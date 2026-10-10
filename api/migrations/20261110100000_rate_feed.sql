-- Rate feed F2: value tiers, board columns, price sources and denomination group mappings (#1051, design docs/design/rate-feed-connector.md §4)

ALTER TABLE rate_reference_batches
    ADD COLUMN source_published_at timestamptz NULL;

ALTER TABLE rate_reference_lines
    ADD COLUMN instrument_key text NULL,
    ADD COLUMN value_tier text NULL,
    ADD COLUMN tier_lower_exclusive_thb numeric(20,2) NULL,
    ADD COLUMN tier_upper_inclusive_thb numeric(20,2) NULL,
    ADD COLUMN base_buy numeric(20,8) NULL,
    ADD COLUMN base_sell numeric(20,8) NULL;

-- The line's identity now includes the board row and the tier
-- Drop existing unique constraint of (batch_id, currency_code, reference_type)
DO $$
DECLARE
    c_name text;
BEGIN
    SELECT conname INTO c_name
    FROM pg_constraint
    WHERE conrelid = 'rate_reference_lines'::regclass AND contype = 'u';

    IF c_name IS NOT NULL THEN
        EXECUTE 'ALTER TABLE rate_reference_lines DROP CONSTRAINT ' || quote_ident(c_name);
    END IF;
END $$;

CREATE UNIQUE INDEX rate_reference_lines_identity
    ON rate_reference_lines (batch_id, currency_code, reference_type,
                             COALESCE(instrument_key, ''), COALESCE(value_tier, ''));

ALTER TABLE published_rate_lines
    ADD COLUMN value_tier text NOT NULL DEFAULT 'ALL',
    ADD COLUMN tier_lower_exclusive_thb numeric(20,2) NULL,
    ADD COLUMN tier_upper_inclusive_thb numeric(20,2) NULL,
    ADD COLUMN base_buy_rate numeric(20,8) NULL,
    ADD COLUMN base_sell_rate numeric(20,8) NULL;

-- Drop existing unique constraint of (publication_id, currency_code, denomination_group)
DO $$
DECLARE
    c_name text;
BEGIN
    SELECT conname INTO c_name
    FROM pg_constraint
    WHERE conrelid = 'published_rate_lines'::regclass AND contype = 'u';

    IF c_name IS NOT NULL THEN
        EXECUTE 'ALTER TABLE published_rate_lines DROP CONSTRAINT ' || quote_ident(c_name);
    END IF;
END $$;

ALTER TABLE published_rate_lines
    ADD CONSTRAINT published_rate_lines_identity UNIQUE (publication_id, currency_code, denomination_group, value_tier);

ALTER TABLE branches
    ADD COLUMN price_source text NOT NULL DEFAULT 'BOT_DRAFT'
        CHECK (price_source IN ('BOT_DRAFT', 'RATE_BOARD'));

CREATE TABLE denomination_group_sources (
    currency_code  char(3) NOT NULL REFERENCES currencies(code),
    group_code     text    NOT NULL,
    provider       text    NOT NULL,
    instrument_key text    NOT NULL,
    updated_by     text    NOT NULL,
    updated_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (currency_code, group_code, provider)
);
