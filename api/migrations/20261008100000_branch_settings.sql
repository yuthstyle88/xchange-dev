CREATE TABLE branch_settings (
    branch_id  text        NOT NULL REFERENCES branches (id),
    name       text        NOT NULL CHECK (name ~ '^[a-z][A-Za-z0-9]{0,39}$'),
    version    bigint      NOT NULL CHECK (version >= 1),
    body       jsonb       NOT NULL,
    updated_at timestamptz NOT NULL,
    updated_by text        NOT NULL,
    PRIMARY KEY (branch_id, name)
);
CREATE TABLE branch_setting_history (
    branch_id text        NOT NULL,
    name      text        NOT NULL,
    version   bigint      NOT NULL,
    actor     text        NOT NULL,
    at        timestamptz NOT NULL,
    before    jsonb       NOT NULL,
    after     jsonb       NOT NULL,
    PRIMARY KEY (branch_id, name, version),
    FOREIGN KEY (branch_id, name) REFERENCES branch_settings (branch_id, name)
);
