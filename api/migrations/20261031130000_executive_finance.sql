-- Executive & Finance (EX-2, #884): branch_versions changed_at and A01/F01 tables

ALTER TABLE branch_versions
    ADD COLUMN changed_at timestamptz NOT NULL DEFAULT now();

CREATE TABLE branch_alert_settings (
    organization_id     uuid NOT NULL,
    branch_id           text NOT NULL,
    stale_data_minutes  integer NULL CHECK (stale_data_minutes > 0),
    version             integer NOT NULL DEFAULT 1 CHECK (version > 0),
    updated_by          uuid NOT NULL,
    updated_at          timestamptz NOT NULL,
    PRIMARY KEY (organization_id, branch_id),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE TABLE branch_stock_floors (
    organization_id     uuid NOT NULL,
    branch_id           text NOT NULL,
    currency            char(3) NOT NULL REFERENCES currencies (code),
    min_quantity        numeric(20, 4) NOT NULL CHECK (min_quantity >= 0),
    updated_by          uuid NOT NULL,
    updated_at          timestamptz NOT NULL,
    PRIMARY KEY (organization_id, branch_id, currency),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE TABLE reconciliation_certifications (
    id                  uuid PRIMARY KEY,
    organization_id     uuid NOT NULL,
    branch_id           text NOT NULL,
    business_date       date NOT NULL,
    figures_hash        text NOT NULL,
    figures             jsonb NOT NULL,
    certified_by        uuid NOT NULL,
    certified_at        timestamptz NOT NULL,
    note                text NULL,
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
);

CREATE INDEX idx_reconciliation_certifications_lookup
    ON reconciliation_certifications (organization_id, branch_id, business_date, certified_at DESC);

CREATE OR REPLACE FUNCTION prevent_reconciliation_certifications_modification()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'reconciliation_certifications is append-only: UPDATE and DELETE are not allowed';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER reconciliation_certifications_no_update_or_delete
BEFORE UPDATE OR DELETE ON reconciliation_certifications
FOR EACH STATEMENT EXECUTE FUNCTION prevent_reconciliation_certifications_modification();
