ALTER TABLE rate_policy_versions
    ADD COLUMN value_tiers_on boolean NOT NULL DEFAULT false;

CREATE TABLE rate_policy_value_tiers (
    policy_version_id   uuid          NOT NULL REFERENCES rate_policy_versions(id),
    tier_no             smallint      NOT NULL CHECK (tier_no BETWEEN 1 AND 4),
    upper_inclusive_thb numeric(20,2) NULL,         -- NULL only for tier 4 (open above)
    buy_margin_pct      numeric(9,4)  NOT NULL,     -- signed, scale 4, like buy_adjust_pct
    sell_margin_pct     numeric(9,4)  NOT NULL,
    PRIMARY KEY (policy_version_id, tier_no),
    CHECK ((tier_no = 4) = (upper_inclusive_thb IS NULL)),
    CHECK (upper_inclusive_thb IS NULL OR upper_inclusive_thb > 0)
);

CREATE TABLE org_starting_prices (
    id              uuid          PRIMARY KEY,
    organization_id uuid          NOT NULL REFERENCES organizations(id),
    currency_code   char(3)       NOT NULL REFERENCES currencies(code),
    version         integer       NOT NULL,
    unit_size       numeric(20,4) NOT NULL,          -- currencies.unit_size when entered
    buy             numeric(20,8) NULL,
    sell            numeric(20,8) NULL,
    entered_by      text          NOT NULL,
    entered_at      timestamptz   NOT NULL DEFAULT now(),
    reason          text          NOT NULL,
    reference_line_id uuid        NOT NULL REFERENCES rate_reference_lines(id),
    UNIQUE (organization_id, currency_code, version),
    CHECK (buy IS NOT NULL OR sell IS NOT NULL),
    CHECK (buy IS NULL OR buy > 0),
    CHECK (sell IS NULL OR sell > 0),
    CHECK (buy IS NULL OR sell IS NULL OR buy <= sell)
);

ALTER TABLE rate_policy_versions ADD COLUMN sharp_move_alert_pct numeric(9,4) NULL
    CHECK (sharp_move_alert_pct IS NULL OR sharp_move_alert_pct > 0);
