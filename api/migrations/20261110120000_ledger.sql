-- LG-1 (#923): Ledger and inventory tables, migration 1.
-- Forward-only migration; dropping these tables loses nothing real because nothing posts until LG-4.

CREATE TABLE cash_accounts (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    branch_id       text NULL,
    currency        char(3) NOT NULL REFERENCES currencies(code),
    kind            text NOT NULL CHECK (kind IN ('DRAWER','SAFE','IN_TRANSIT','CUSTOMER','BANK','VARIANCE')),
    shift_id        text NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'DRAWER') = (shift_id IS NOT NULL)),
    UNIQUE NULLS NOT DISTINCT (organization_id, branch_id, currency, kind, shift_id)
);

CREATE TABLE ledger_postings (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    branch_id       text NOT NULL,
    source_type     text NOT NULL CHECK (source_type IN ('EXCHANGE','TRANSFER','COUNT','OPENING','REVERSAL','ADJUSTMENT')),
    source_id       text NOT NULL,
    source_key      text NOT NULL,
    status          text NOT NULL CHECK (status IN ('POSTED')),
    posted_at       timestamptz NOT NULL,
    posted_by       text NOT NULL,
    UNIQUE (organization_id, source_type, source_id, source_key)
);

CREATE TABLE ledger_entries (
    id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    posting_id      uuid NOT NULL REFERENCES ledger_postings(id),
    account_id      uuid NOT NULL REFERENCES cash_accounts(id),
    currency        char(3) NOT NULL REFERENCES currencies(code),
    quantity_signed numeric(20,4) NOT NULL CHECK (quantity_signed <> 0),
    valuation_thb   numeric(20,2) NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ledger_entries_account ON ledger_entries (account_id, id);
CREATE INDEX ledger_entries_posting_id ON ledger_entries (posting_id);

CREATE OR REPLACE FUNCTION check_ledger_posting_balanced()
RETURNS trigger AS $$
DECLARE
    target_id uuid;
    unbalanced_currency text;
    unbalanced_sum numeric;
BEGIN
    FOR target_id IN
        SELECT DISTINCT p FROM (
            SELECT NEW.posting_id AS p WHERE NEW.posting_id IS NOT NULL
            UNION
            SELECT OLD.posting_id AS p WHERE OLD.posting_id IS NOT NULL
        ) sub
    LOOP
        SELECT currency, SUM(quantity_signed)
        INTO unbalanced_currency, unbalanced_sum
        FROM ledger_entries
        WHERE posting_id = target_id
        GROUP BY currency
        HAVING SUM(quantity_signed) <> 0
        LIMIT 1;

        IF unbalanced_currency IS NOT NULL THEN
            RAISE EXCEPTION 'ledger posting % is not balanced for currency % (sum = %)',
                target_id, unbalanced_currency, unbalanced_sum;
        END IF;
    END LOOP;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER ledger_posting_balanced
AFTER INSERT OR UPDATE OR DELETE ON ledger_entries
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION check_ledger_posting_balanced();

CREATE TABLE inventory_positions (
    organization_id uuid NOT NULL REFERENCES organizations(id),
    branch_id       text NOT NULL,
    currency        char(3) NOT NULL REFERENCES currencies(code),
    quantity        numeric(20,4) NOT NULL,
    cost_thb        numeric(20,2) NOT NULL,
    version         bigint NOT NULL DEFAULT 1,
    PRIMARY KEY (organization_id, branch_id, currency)
);

CREATE TABLE inventory_lots (
    id                uuid PRIMARY KEY,
    organization_id   uuid NOT NULL REFERENCES organizations(id),
    branch_id         text NOT NULL,
    currency          char(3) NOT NULL REFERENCES currencies(code),
    denomination_id   bigint NULL REFERENCES denominations(id),
    acquired_at       timestamptz NOT NULL,
    quantity          numeric(20,4) NOT NULL CHECK (quantity > 0),
    unit_cost_thb     numeric(20,8) NOT NULL CHECK (unit_cost_thb >= 0),
    source_posting_id uuid NOT NULL REFERENCES ledger_postings(id)
);

CREATE TABLE inventory_movements (
    id                  bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    organization_id     uuid NOT NULL REFERENCES organizations(id),
    branch_id           text NOT NULL,
    currency            char(3) NOT NULL REFERENCES currencies(code),
    lot_id              uuid NULL REFERENCES inventory_lots(id),
    posting_id          uuid NOT NULL REFERENCES ledger_postings(id),
    transaction_line_id uuid NULL,
    movement_type       text NOT NULL CHECK (movement_type IN ('ACQUIRE','DISPOSE','TRANSFER_OUT','TRANSFER_IN','ADJUST')),
    quantity_signed     numeric(20,4) NOT NULL CHECK (quantity_signed <> 0 OR movement_type = 'ADJUST'),
    cost_thb_signed     numeric(20,2) NOT NULL,
    average_after       numeric(20,8) NULL,
    created_at          timestamptz NOT NULL
);

CREATE TABLE cash_limits (
    organization_id            uuid NOT NULL REFERENCES organizations(id),
    branch_id                  text NOT NULL,
    count_difference_limit_thb numeric(20,2) NULL CHECK (count_difference_limit_thb IS NULL OR count_difference_limit_thb >= 0),
    updated_by                 text NOT NULL,
    updated_at                 timestamptz NOT NULL,
    PRIMARY KEY (organization_id, branch_id)
);
