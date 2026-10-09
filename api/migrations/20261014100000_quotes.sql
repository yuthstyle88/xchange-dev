-- 20261014100000_quotes.sql
-- TX-1: quotes, quote_lines, exchange_settings, branch_exchange_limits tables

CREATE TABLE quotes (
    id                         uuid PRIMARY KEY,
    organization_id            uuid NOT NULL REFERENCES organizations(id),
    branch_id                  text NOT NULL,
    customer_id                text NULL REFERENCES customers(id),
    type                       text NOT NULL CHECK (type IN ('INDICATIVE', 'BINDING')) CHECK (type = 'INDICATIVE'),
    channel                    text NOT NULL CHECK (channel IN ('DESK', 'KIOSK', 'BOOKING', 'APP')),
    status                     text NOT NULL CHECK (status IN ('ACTIVE', 'ACCEPTED', 'CANCELLED')),
    publication_id             uuid NOT NULL REFERENCES rate_publications(id),
    expires_at                 timestamptz NOT NULL,
    benefit_snapshot           jsonb NOT NULL DEFAULT '{}',
    created_by                 text NOT NULL,
    accepted_by_transaction_id uuid NULL,
    created_at                 timestamptz NOT NULL,
    updated_at                 timestamptz NOT NULL,
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id),
    CHECK ((status = 'ACCEPTED') = (accepted_by_transaction_id IS NOT NULL))
);

CREATE TABLE quote_lines (
    id                     uuid PRIMARY KEY,
    organization_id        uuid NOT NULL REFERENCES organizations(id),
    quote_id               uuid NOT NULL REFERENCES quotes(id) ON DELETE CASCADE,
    published_rate_line_id uuid NOT NULL REFERENCES published_rate_lines(id),
    shop_side              text NOT NULL CHECK (shop_side IN ('BUY', 'SELL')),
    currency               char(3) NOT NULL REFERENCES currencies(code),
    denomination_group     text NOT NULL DEFAULT 'ALL',
    quantity               numeric(20,4) NOT NULL CHECK (quantity > 0),
    unit_size              numeric(20,4) NOT NULL CHECK (unit_size > 0),
    base_rate              numeric(20,8) NOT NULL CHECK (base_rate > 0),
    final_rate             numeric(20,8) NOT NULL CHECK (final_rate > 0),
    thb_amount             numeric(20,2) NOT NULL CHECK (thb_amount >= 0),
    fee_thb                numeric(20,2) NOT NULL DEFAULT 0 CHECK (fee_thb >= 0 AND fee_thb <= thb_amount),
    settle_thb             numeric(20,2) NOT NULL CHECK (settle_thb >= 0),
    line_no                integer NOT NULL,
    created_at             timestamptz NOT NULL,
    updated_at             timestamptz NOT NULL,
    UNIQUE (quote_id, line_no)
);

CREATE TABLE exchange_settings (
    organization_id   uuid PRIMARY KEY REFERENCES organizations(id),
    quote_ttl_seconds integer NOT NULL DEFAULT 300 CHECK (quote_ttl_seconds BETWEEN 30 AND 3600),
    updated_by        text NOT NULL,
    updated_at        timestamptz NOT NULL
);

CREATE TABLE branch_exchange_limits (
    organization_id     uuid NOT NULL REFERENCES organizations(id),
    branch_id           text NOT NULL,
    approval_limit_thb  numeric(20,2) NULL CHECK (approval_limit_thb IS NULL OR approval_limit_thb > 0),
    updated_by          text NOT NULL,
    updated_at          timestamptz NOT NULL,
    PRIMARY KEY (organization_id, branch_id),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id)
);

-- Backfill: one exchange_settings row per organisation with 300 s
INSERT INTO exchange_settings (organization_id, quote_ttl_seconds, updated_by, updated_at)
SELECT id, 300, 'migration', now() FROM organizations;
