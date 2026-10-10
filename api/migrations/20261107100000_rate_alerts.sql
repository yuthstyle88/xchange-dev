-- 20261106110000_rate_alerts.sql
-- AL-1: rate_alert_settings, rate_alerts, rate_alert_events (design rate-alerts.md §4.1)
--
-- Forward-only migration. Dropping these three tables would lose all alert settings,
-- alerts, and alert events (test data only in v3 before first install).

CREATE TABLE rate_alert_settings (
    organization_id          uuid PRIMARY KEY REFERENCES organizations(id),
    default_cooldown_seconds integer NOT NULL CHECK (default_cooldown_seconds >= 0),
    max_active_per_customer  integer NOT NULL CHECK (max_active_per_customer > 0),
    default_lifetime_days    integer NULL CHECK (default_lifetime_days > 0),
    observe_policy           text NOT NULL CHECK (observe_policy IN ('ALL_FRESH', 'ALL', 'AUTO_FRESH')),
    updated_by               uuid NOT NULL REFERENCES users(id),
    updated_at               timestamptz NOT NULL
);

CREATE TABLE rate_alerts (
    id                       uuid PRIMARY KEY,
    organization_id          uuid NOT NULL REFERENCES organizations(id),
    customer_id              text NOT NULL REFERENCES customers(id),
    branch_id                text NOT NULL,
    currency_code            text NOT NULL REFERENCES currencies(code),
    shop_side                text NOT NULL CHECK (shop_side IN ('BUY', 'SELL')),
    unit_size                numeric(20,4) NOT NULL CHECK (unit_size > 0),
    condition                text NOT NULL CHECK (condition IN ('ABOVE', 'BELOW')),
    target                   numeric(20,8) NOT NULL CHECK (target > 0),
    enabled                  boolean NOT NULL DEFAULT true,
    cooldown_seconds         integer NOT NULL CHECK (cooldown_seconds >= 0),
    expires_at               timestamptz NULL,
    last_triggered_at        timestamptz NULL,
    relation                 text NOT NULL DEFAULT 'UNKNOWN' CHECK (relation IN ('UNKNOWN', 'SATISFIED', 'NOT_SATISFIED')),
    last_publication_version integer NULL,
    episode_no               integer NOT NULL DEFAULT 0,
    disabled_reason          text NULL CHECK (disabled_reason IN ('UNIT_CHANGED')),
    deleted_at               timestamptz NULL,
    version                  integer NOT NULL DEFAULT 1,
    idempotency_key          text NOT NULL,
    created_at               timestamptz NOT NULL DEFAULT now(),
    updated_at               timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id),
    UNIQUE (customer_id, idempotency_key)
);

CREATE INDEX rate_alerts_watch ON rate_alerts (branch_id, currency_code, shop_side)
    WHERE enabled AND deleted_at IS NULL;

CREATE INDEX rate_alerts_customer ON rate_alerts (customer_id)
    WHERE deleted_at IS NULL;

CREATE TABLE rate_alert_events (
    id                     uuid PRIMARY KEY,
    organization_id        uuid NOT NULL REFERENCES organizations(id),
    alert_id               uuid NOT NULL REFERENCES rate_alerts(id),
    published_rate_line_id uuid NOT NULL REFERENCES published_rate_lines(id),
    observed_rate          numeric(20,8) NOT NULL,
    trigger_at             timestamptz NOT NULL,
    episode_no             integer NOT NULL CHECK (episode_no > 0),
    created_at             timestamptz NOT NULL DEFAULT now(),
    UNIQUE (alert_id, episode_no),
    UNIQUE (alert_id, published_rate_line_id)
);
