CREATE TABLE files (
    key        text PRIMARY KEY,
    branch_id  text NOT NULL REFERENCES branches (id),
    owner_id   text NOT NULL,
    mime       text NOT NULL,
    sha256     text NOT NULL,
    created_at timestamptz NOT NULL
);
