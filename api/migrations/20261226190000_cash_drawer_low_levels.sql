CREATE TABLE cash_drawer_low_levels (
    organization_id uuid NOT NULL,
    branch_id text NOT NULL,
    currency char(3) NOT NULL REFERENCES currencies (code),
    low_level numeric(20,4) NOT NULL CHECK (low_level > 0),
    updated_by uuid NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (organization_id, branch_id, currency),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);
