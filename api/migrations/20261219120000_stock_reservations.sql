-- 20261219120000_stock_reservations.sql
-- B05 SR-0: stock_reservations table

CREATE TABLE stock_reservations (
    id               uuid PRIMARY KEY,
    organization_id  uuid NOT NULL REFERENCES organizations(id),
    branch_id        text NOT NULL,
    quote_line_id    uuid NOT NULL REFERENCES quote_lines(id),
    currency         char(3) NOT NULL REFERENCES currencies(code),
    denomination_id  bigint NULL,
    quantity         numeric(20,4) NOT NULL CHECK (quantity > 0),
    expires_at       timestamptz NOT NULL,
    status           text NOT NULL CHECK (status IN ('RESERVED','CONSUMED','RELEASED')),
    transaction_id   uuid NULL,
    released_reason  text NULL CHECK (released_reason IN ('EXPIRED','CANCELLED')),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id),
    CHECK ((status = 'CONSUMED') = (transaction_id IS NOT NULL)),
    CHECK ((status = 'RELEASED') = (released_reason IS NOT NULL))
);

CREATE UNIQUE INDEX one_live_reservation_per_quote_line ON stock_reservations (quote_line_id)
    WHERE status = 'RESERVED';

CREATE INDEX stock_reservations_active ON stock_reservations (organization_id, branch_id, currency, expires_at)
    WHERE status = 'RESERVED';
