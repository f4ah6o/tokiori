-- P1: bitemporal history.
--
-- Facts are never overwritten. A change closes the version that was current
-- and inserts a new one in the same revision, so an older revision keeps
-- answering exactly as it did before the change.

CREATE TABLE scope_revisions (
    scope_id       TEXT        NOT NULL,
    last_revision  BIGINT      NOT NULL CHECK (last_revision >= 0),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_id)
);

-- One version of one attribute of one entity.
--
--   [valid_from, valid_until)      when the value was in effect
--   [recorded_from, recorded_to)   which revisions knew it as current
--
-- `recorded_to` is the revision that replaced the version. A query at that
-- revision sees the replacement, so the interval is inclusive at both ends
-- for the purpose of choosing candidates.
CREATE TABLE fact_versions (
    scope_id      TEXT        NOT NULL,
    entity_type   TEXT        NOT NULL,
    entity_id     TEXT        NOT NULL,
    attribute     TEXT        NOT NULL,
    valid_from    DATE        NOT NULL,
    valid_until   DATE,
    recorded_from BIGINT      NOT NULL CHECK (recorded_from >= 1),
    recorded_to   BIGINT,
    value         JSONB       NOT NULL,
    PRIMARY KEY (scope_id, entity_type, entity_id, attribute, recorded_from, valid_from),
    CONSTRAINT fact_versions_valid_interval
        CHECK (valid_until IS NULL OR valid_until > valid_from),
    CONSTRAINT fact_versions_recorded_interval
        CHECK (recorded_to IS NULL OR recorded_to > recorded_from)
);

CREATE INDEX fact_versions_lookup
    ON fact_versions (scope_id, entity_type, entity_id, attribute, recorded_from);

-- One version of one relation instance, including its role participants.
CREATE TABLE relation_versions (
    scope_id      TEXT        NOT NULL,
    relation_type TEXT        NOT NULL,
    relation_id   TEXT        NOT NULL,
    valid_from    DATE        NOT NULL,
    valid_until   DATE,
    recorded_from BIGINT      NOT NULL CHECK (recorded_from >= 1),
    recorded_to   BIGINT,
    attributes    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (scope_id, relation_type, relation_id, recorded_from, valid_from),
    CONSTRAINT relation_versions_valid_interval
        CHECK (valid_until IS NULL OR valid_until > valid_from),
    CONSTRAINT relation_versions_recorded_interval
        CHECK (recorded_to IS NULL OR recorded_to > recorded_from)
);

CREATE INDEX relation_versions_lookup
    ON relation_versions (scope_id, relation_type, relation_id, recorded_from);

-- Participants of one relation version. They share the version's recorded
-- interval, so a participant change is a new version, not an edit.
CREATE TABLE relation_role_versions (
    scope_id                TEXT   NOT NULL,
    relation_type           TEXT   NOT NULL,
    relation_id             TEXT   NOT NULL,
    recorded_from           BIGINT NOT NULL,
    valid_from              DATE   NOT NULL,
    role                    TEXT   NOT NULL,
    participant_entity_type TEXT   NOT NULL,
    participant_id          TEXT   NOT NULL,
    PRIMARY KEY (scope_id, relation_type, relation_id, recorded_from, valid_from, role),
    FOREIGN KEY (scope_id, relation_type, relation_id, recorded_from, valid_from)
        REFERENCES relation_versions (scope_id, relation_type, relation_id, recorded_from, valid_from)
        ON DELETE CASCADE
);

CREATE INDEX relation_role_versions_by_participant
    ON relation_role_versions (scope_id, participant_entity_type, participant_id);
