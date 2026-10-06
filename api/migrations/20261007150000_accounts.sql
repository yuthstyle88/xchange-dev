CREATE TABLE owners (
    id            text        PRIMARY KEY,
    username      text        NOT NULL UNIQUE,
    name          text        NOT NULL,
    password_hash text        NOT NULL,
    active        boolean     NOT NULL DEFAULT true,
    version       integer     NOT NULL DEFAULT 1,
    created_at    timestamptz NOT NULL
);

CREATE TABLE owner_sessions (
    hash     text        PRIMARY KEY,
    owner_id text        NOT NULL REFERENCES owners(id) ON DELETE CASCADE,
    version  integer     NOT NULL,
    expires  timestamptz NOT NULL
);

CREATE TABLE employees (
    id            text        PRIMARY KEY,
    username      text        NOT NULL UNIQUE,
    name          text        NOT NULL,
    role          text        NOT NULL,
    branch_id     text        NOT NULL,
    password_hash text        NOT NULL,
    active        boolean     NOT NULL DEFAULT true,
    must_change   boolean     NOT NULL DEFAULT true,
    version       integer     NOT NULL DEFAULT 1,
    created_at    timestamptz NOT NULL
);

CREATE TABLE employee_sessions (
    hash        text        PRIMARY KEY,
    employee_id text        NOT NULL REFERENCES employees(id) ON DELETE CASCADE,
    version     integer     NOT NULL,
    expires     timestamptz NOT NULL
);

CREATE TABLE employee_audit (
    id      bigserial   PRIMARY KEY,
    actor   text        NOT NULL,
    subject text        NOT NULL,
    action  text        NOT NULL,
    at      timestamptz NOT NULL
);

CREATE TABLE auth_attempts (
    key          text        NOT NULL,
    window_start timestamptz NOT NULL,
    count        integer     NOT NULL DEFAULT 0,
    PRIMARY KEY (key, window_start)
);

CREATE INDEX idx_employees_branch_id ON employees (branch_id);
CREATE INDEX idx_employee_sessions_employee_id ON employee_sessions (employee_id);
CREATE INDEX idx_owner_sessions_owner_id ON owner_sessions (owner_id);
