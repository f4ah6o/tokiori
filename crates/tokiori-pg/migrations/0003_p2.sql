-- P2: operations and their events.
--
-- An operation records *why* state changed: who asked, under which request,
-- and what the system decided. The state itself stays in the P1 history;
-- operations are the audit trail, not the source of truth.

CREATE TABLE operations (
    scope_id       TEXT        NOT NULL,
    operation_type TEXT        NOT NULL,
    -- The caller's key. Unique per scope and operation type, so the same
    -- request replayed twice is recognised rather than applied twice.
    idempotency_key TEXT       NOT NULL,
    -- Digest of the normalized input. Two requests with the same key but a
    -- different digest are a conflict, not a retry.
    input_digest   TEXT        NOT NULL,
    revision       BIGINT      NOT NULL CHECK (revision >= 1),
    actor          TEXT        NOT NULL,
    reason         TEXT        NOT NULL,
    -- The recorded result, so a replay returns what the first call returned.
    result         JSONB       NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_id, operation_type, idempotency_key)
);

CREATE INDEX operations_by_revision ON operations (scope_id, revision);

-- Events are written in the same transaction as the state change and the
-- operation row. Delivery is at-least-once: a consumer must deduplicate on
-- event_id, and an event is only retired once it has been acknowledged.
CREATE TABLE outbox_events (
    event_id       UUID        PRIMARY KEY,
    scope_id       TEXT        NOT NULL,
    revision       BIGINT      NOT NULL CHECK (revision >= 1),
    event_type     TEXT        NOT NULL,
    payload        JSONB       NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at     TIMESTAMPTZ,
    acknowledged_at TIMESTAMPTZ
);

CREATE INDEX outbox_events_pending
    ON outbox_events (scope_id, created_at)
    WHERE acknowledged_at IS NULL;
