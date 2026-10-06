CREATE TABLE branch_versions (
    branch_id text   PRIMARY KEY,
    version   bigint NOT NULL DEFAULT 0 CHECK (version >= 0)
);
