# ADR 0006: Operations as auditable intent

- Status: accepted (P2)
- Context: [ROADMAP.md](../ROADMAP.md) §5 gate 3, [ADR 0005](0005-p1-bitemporal-engine.md)

## Decision

An operation records **why** state changed; the P1 history remains the truth
about **what** the state is. Operations are not an event-sourced log that
rebuilds state -- the roadmap lists that as a non-goal -- they are the audit
trail that makes a change explainable.

An operation commits four things together or none of them:

1. the revision it was recorded under,
2. the state change,
3. the ledger row (key, input digest, actor, reason, result),
4. the outbox event.

## Idempotency

The key is `(scope_id, operation_type, idempotency_key)`. The same string used
for a transfer and a cancel, or in two scopes, is three different requests.

Whether a repeated key is a replay or a conflict is decided by a digest of the
**normalized input**, not by the key alone:

| Same key | Same input | Result |
| --- | --- | --- |
| yes | yes | the original result, and nothing is applied |
| yes | no | `idempotency_conflict`, and nothing is applied |
| no | - | a new operation |

Actor and reason are metadata about the request, so they are not part of the
digest: the same request retried by a different operator is still the same
request.

`RETURNING` on the insert distinguishes the call that created the row from a
replay. Without it, a replay and a fresh record look identical afterwards.

## Cancellation is compensation

A cancellation does not delete the transfer. It records a new version that
restores the previous department, so both the transfer and its undoing remain
readable, and the transfer's own revision still answers with the move.

## Delivery is at-least-once

`claim_outbox` does not remove an event; it stays pending until acknowledged.
A consumer that crashes between delivery and acknowledgement sees the event
again with the same `event_id` and must deduplicate on it. **Exactly-once is
not claimed**, matching the roadmap's non-goals.

## Consequences

- A rejected precondition records no operation, so the same request id can be
  retried after the cause is fixed.
- Fault injection at the event insert and at the ledger insert rolls back the
  state change too; the tests force both.
- The effect shares the operation's revision. Letting the role write allocate
  its own number put the effect in a later revision than the operation
  claimed, which broke queries at the operation's revision.
