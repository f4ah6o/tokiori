-- P0 storage: declared models, typed entities, and role-bound relations.
--
-- Every key carries scope_id so a future tenant boundary never needs a
-- backfill that guesses which tenant an existing fact belonged to.

CREATE TABLE models (
    scope_id    TEXT        NOT NULL,
    model       TEXT        NOT NULL,
    version     INTEGER     NOT NULL CHECK (version >= 1),
    time_zone   TEXT        NOT NULL,
    definition  JSONB       NOT NULL,
    digest      TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_id, model, version)
);

CREATE TABLE entities (
    scope_id      TEXT        NOT NULL,
    model         TEXT        NOT NULL,
    model_version INTEGER     NOT NULL,
    entity_type   TEXT        NOT NULL,
    id            TEXT        NOT NULL,
    attributes    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_id, entity_type, id),
    FOREIGN KEY (scope_id, model, model_version)
        REFERENCES models (scope_id, model, version)
);

CREATE INDEX entities_by_scope_type ON entities (scope_id, entity_type);

CREATE TABLE relations (
    scope_id      TEXT        NOT NULL,
    model         TEXT        NOT NULL,
    model_version INTEGER     NOT NULL,
    relation_type TEXT        NOT NULL,
    id            TEXT        NOT NULL,
    attributes    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    valid_from    DATE        NOT NULL,
    valid_until   DATE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_id, relation_type, id),
    CONSTRAINT relations_valid_interval CHECK (valid_until IS NULL OR valid_until > valid_from),
    FOREIGN KEY (scope_id, model, model_version)
        REFERENCES models (scope_id, model, version)
);

CREATE INDEX relations_by_scope_type ON relations (scope_id, relation_type, valid_from);

CREATE TABLE relation_roles (
    scope_id                TEXT NOT NULL,
    relation_type           TEXT NOT NULL,
    relation_id             TEXT NOT NULL,
    role                    TEXT NOT NULL,
    participant_entity_type TEXT NOT NULL,
    participant_id          TEXT NOT NULL,
    PRIMARY KEY (scope_id, relation_type, relation_id, role),
    FOREIGN KEY (scope_id, relation_type, relation_id)
        REFERENCES relations (scope_id, relation_type, id)
        ON DELETE CASCADE
);

CREATE INDEX relation_roles_by_participant
    ON relation_roles (scope_id, participant_entity_type, participant_id);
