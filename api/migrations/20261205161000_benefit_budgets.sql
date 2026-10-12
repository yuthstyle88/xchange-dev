CREATE TABLE coupons (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    rule_id         uuid NOT NULL REFERENCES benefit_rules(id),
    code_hash       bytea NOT NULL,
    customer_id     text NULL REFERENCES customers(id),
    expires_at      timestamptz NOT NULL,
    status          text NOT NULL CHECK (status IN ('ISSUED','REDEEMED','VOID')),
    issued_by       uuid NOT NULL REFERENCES users(id),
    issued_at       timestamptz NOT NULL,
    UNIQUE (organization_id, code_hash)
);

CREATE TABLE campaign_budgets (
    campaign_version_id uuid PRIMARY KEY REFERENCES campaign_versions(id),
    consumed_thb        numeric(20,2) NOT NULL DEFAULT 0 CHECK (consumed_thb >= 0),
    used_count          int NOT NULL DEFAULT 0 CHECK (used_count >= 0)
);

CREATE TABLE benefit_reservations (
    id                  uuid PRIMARY KEY,
    organization_id     uuid NOT NULL REFERENCES organizations(id),
    rule_id             uuid NOT NULL REFERENCES benefit_rules(id),
    campaign_version_id uuid NOT NULL REFERENCES campaign_versions(id),
    quote_id            uuid NOT NULL REFERENCES quotes(id),
    coupon_id           uuid NULL REFERENCES coupons(id),
    customer_id         text NULL REFERENCES customers(id),
    amount_thb          numeric(20,2) NOT NULL CHECK (amount_thb >= 0),
    status              text NOT NULL CHECK (status IN ('RESERVED','CONSUMED','RELEASED')),
    expires_at          timestamptz NOT NULL,
    created_at          timestamptz NOT NULL,
    UNIQUE (quote_id, rule_id)
);

CREATE UNIQUE INDEX one_live_reservation_per_coupon ON benefit_reservations (coupon_id)
    WHERE status = 'RESERVED' AND coupon_id IS NOT NULL;

CREATE TABLE benefit_redemptions (
    id                      uuid PRIMARY KEY,
    organization_id         uuid NOT NULL REFERENCES organizations(id),
    rule_id                 uuid NOT NULL REFERENCES benefit_rules(id),
    campaign_version_id     uuid NOT NULL REFERENCES campaign_versions(id),
    reservation_id          uuid NOT NULL UNIQUE REFERENCES benefit_reservations(id),
    coupon_id               uuid NULL REFERENCES coupons(id),
    transaction_id          uuid NOT NULL REFERENCES transactions(id),
    customer_id             text NULL REFERENCES customers(id),
    referral_attribution_id uuid NULL REFERENCES referral_attributions(id),
    benefit_class           text NOT NULL CHECK (benefit_class IN ('PRICE','REWARD')),
    amount_thb              numeric(20,2) NOT NULL CHECK (amount_thb >= 0),
    status                  text NOT NULL CHECK (status IN ('REDEEMED','REVERSED')),
    reversal_id             uuid NULL REFERENCES reversals(id),
    redeemed_at             timestamptz NOT NULL,
    UNIQUE (rule_id, transaction_id),
    CHECK ((status = 'REVERSED') = (reversal_id IS NOT NULL))
);
