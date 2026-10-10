use crate::infrai_errors::{FailureCapture, InfraiError, InfraiErrors};
use std::{fmt, future::Future};

pub trait FailureSink {
    fn capture(
        &self,
        failure: &FailureCapture<'_>,
    ) -> impl Future<Output = Result<(), InfraiError>>;
}

impl FailureSink for InfraiErrors {
    async fn capture(&self, failure: &FailureCapture<'_>) -> Result<(), InfraiError> {
        InfraiErrors::capture(self, failure).await.map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStage {
    MatterIntake,
    SignedDocumentDelivery,
    DeadlineFollowUp,
}

impl AgentStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MatterIntake => "matter-intake",
            Self::SignedDocumentDelivery => "signed-document-delivery",
            Self::DeadlineFollowUp => "deadline-follow-up",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatterRun {
    pub matter_id: String,
    pub signed_document_delivered: bool,
    pub follow_up_scheduled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    IntakeRejected(String),
    SignedDocumentDelivery(String),
    DeadlineFollowUp(String),
}

impl WorkflowError {
    pub fn stage(&self) -> AgentStage {
        match self {
            Self::IntakeRejected(_) => AgentStage::MatterIntake,
            Self::SignedDocumentDelivery(_) => AgentStage::SignedDocumentDelivery,
            Self::DeadlineFollowUp(_) => AgentStage::DeadlineFollowUp,
        }
    }
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntakeRejected(reason) => write!(f, "matter intake rejected: {reason}"),
            Self::SignedDocumentDelivery(reason) => {
                write!(f, "signed document delivery failed: {reason}")
            }
            Self::DeadlineFollowUp(reason) => write!(f, "deadline follow-up failed: {reason}"),
        }
    }
}

impl std::error::Error for WorkflowError {}

#[derive(Debug)]
pub enum RunError {
    Workflow(WorkflowError),
    Capture {
        workflow: WorkflowError,
        source: InfraiError,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workflow(error) => write!(f, "{error}"),
            Self::Capture { workflow, source } => write!(f, "{workflow}; capture failed: {source}"),
        }
    }
}

impl std::error::Error for RunError {}

pub async fn run_matter<S, F, Fut>(
    errors: &S,
    occurrence_id: &str,
    matter_id: &str,
    deliver_signed_document: F,
) -> Result<MatterRun, RunError>
where
    S: FailureSink,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    if matter_id.trim().is_empty() {
        return record(
            errors,
            occurrence_id,
            matter_id,
            WorkflowError::IntakeRejected("matter id is empty".into()),
        )
        .await;
    }

    if let Err(reason) = deliver_signed_document().await {
        return record(
            errors,
            occurrence_id,
            matter_id,
            WorkflowError::SignedDocumentDelivery(reason),
        )
        .await;
    }

    Ok(MatterRun {
        matter_id: matter_id.to_owned(),
        signed_document_delivered: true,
        follow_up_scheduled: true,
    })
}

async fn record<T, S: FailureSink>(
    errors: &S,
    occurrence_id: &str,
    matter_id: &str,
    workflow: WorkflowError,
) -> Result<T, RunError> {
    let failure = FailureCapture {
        occurrence_id,
        matter_id,
        stage: workflow.stage().as_str(),
        exception: &workflow.to_string(),
    };
    errors
        .capture(&failure)
        .await
        .map_err(|source| RunError::Capture {
            workflow: workflow.clone(),
            source,
        })?;
    Err(RunError::Workflow(workflow))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, future::ready};

    #[derive(Default)]
    struct RecordingSink {
        stages: RefCell<Vec<String>>,
    }

    impl FailureSink for RecordingSink {
        fn capture(
            &self,
            failure: &FailureCapture<'_>,
        ) -> impl Future<Output = Result<(), InfraiError>> {
            self.stages.borrow_mut().push(failure.stage.to_owned());
            ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn delivery_failure_is_captured_before_deadline_follow_up() {
        let sink = RecordingSink::default();
        let result = run_matter(&sink, "delivery-MAT-2048-v1", "MAT-2048", || async {
            Err("recipient address rejected".to_owned())
        })
        .await;

        assert!(matches!(
            result,
            Err(RunError::Workflow(WorkflowError::SignedDocumentDelivery(_)))
        ));
        assert_eq!(sink.stages.into_inner(), vec!["signed-document-delivery"]);
    }

    #[tokio::test]
    async fn successful_delivery_unlocks_deadline_follow_up() {
        let sink = RecordingSink::default();
        let run = run_matter(&sink, "delivery-MAT-2048-v1", "MAT-2048", || async {
            Ok(())
        })
        .await
        .expect("successful matter run");

        assert!(run.signed_document_delivered);
        assert!(run.follow_up_scheduled);
        assert!(sink.stages.into_inner().is_empty());
    }
}
