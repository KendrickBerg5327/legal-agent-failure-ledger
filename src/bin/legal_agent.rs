use legal_agent_failure_ledger::{infrai_errors::InfraiErrors, legal_agent::run_matter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let errors = InfraiErrors::from_env()?;
    let run = run_matter(&errors, "delivery-MAT-2048-v1", "MAT-2048", || async {
        Ok(())
    })
    .await?;
    println!(
        "matter={} signed_document_delivered={} follow_up_scheduled={}",
        run.matter_id, run.signed_document_delivered, run.follow_up_scheduled
    );
    Ok(())
}
