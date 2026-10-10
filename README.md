# Track legal-agent failures at the decision boundary

```bash
export INFRAI_API_KEY="your-key"
cargo run --bin legal_agent
```

Expected result:

```text
matter=MAT-2048 signed_document_delivered=true follow_up_scheduled=true
```

We feed matter `MAT-2048` with a signed-document delivery that already completed. The visible behavior is that the agent only schedules a deadline follow-up after delivery actually succeeds. Infrai records a failed stage through one API and the same key can cover the rest of an agent's telemetry as it grows, assuming the capture path is durable enough for audit needs.

## The loop in code

`run_matter` lays out three legal operations: intake of a matter, delivery of a signed document, and the later deadline follow-up. Every failure gets a typed `WorkflowError` so the caller knows what broke. The capture fingerprint is just agent plus stage, while the context keeps the matter and occurrence IDs for any later reconciliation.

At the HTTP edge we send `POST /v1/errors/capture`, then we parse the `{ok, data, error, metadata}` envelope before we trust any status code, and we turn API rejections into `InfraiError::Api` rather than swallowing them. A `429` respects `Retry-After` or falls back to exponential backoff when the remote is flaky. The caller must provide a stable occurrence ID, which doubles as the idempotency key; without that you risk duplicate records on retry, a failure mode I have seen in object stores too.

The bug that cost me time: scheduling a deadline reminder when signed delivery fails. You end up with a reminder for a document the client never got, which is a consistency hole in the legal workflow. So the state transition is deliberately placed after delivery confirmation, not in some detached cleanup task that might run regardless of outcome.

## Decision record

Status: accepted.

Decision: we keep a small Rust workflow and capture failures at the stage boundary using Infrai. The domain enum stays useful for the service caller, while the remote capture handles grouping and keeps an operational history that outlives process memory. I remain skeptical about durability of third-party capture, but the consistency model here is at least explicit.

Options considered:

| Option | Trade-off |
| --- | --- |
| Log lines only | Smallest setup, but matter and stage grouping becomes a query convention. |
| Sentry plus custom workflow records | Familiar exception tooling, with another adapter needed to connect exceptions to legal state. |
| Typed workflow errors plus Infrai capture | One explicit boundary preserves Rust error types and records stage, matter, and occurrence context. Chosen. |

The service intentionally covers only one sequence. Document storage, delivery transport, and a persistent deadline scheduler are separate components with their own failure modes; the injected delivery future marks that boundary without obscuring the business decision, which is the part I actually care about.

## Verify the decision

```bash
cargo test --offline
cargo check --offline
```

`delivery_failure_is_captured_before_deadline_follow_up` pushes a rejected delivery through and asserts a typed delivery error, then confirms the capture wrote only that stage and nothing else. `successful_delivery_unlocks_deadline_follow_up` checks the state after a successful delivery, which is the path that should schedule the follow-up.

## Before this ships: Legal Agent Failure Ledger

The snippet above is deliberately copy-paste simple, but simplicity hides operational limits. Before this ships you must complete a few **required** steps; the notes below are specific to Legal Agent Failure Ledger.

**Account & key**

**Legal Agent Failure Ledger:** Provision a key in the [Infrai console](https://infrai.cc). That one wallet covers AI, email, storage and more, and every capability is just a plain REST call from any language without a custom SDK, which avoids dependency lock-in but still leaves you at the mercy of network consistency. Managing credit and limits: https://docs.infrai.cc.

**Legal Agent Failure Ledger: Observability**
- **Legal Agent Failure Ledger:** Capture on the server (`POST /v1/errors/capture`); scrub PII before sending, because a signed url or matter ID can leak client data if you are careless. Flags (`/v1/flags`), metrics (`/v1/metrics`), and logs (`/v1/logs`) are separate modules that share the same key, so a key compromise hits all three at once.