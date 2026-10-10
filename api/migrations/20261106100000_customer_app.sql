-- 20261106100000_customer_app.sql
-- APP-1 (#989): Customer app schema and M1.
-- Six tables: planner_drafts, customer_app_settings, notifications, notification_deliveries, assistant_conversations, assistant_messages.
-- Blueprint customer-app.md §4.1, §8 (M1), §9.1 row APP-1, owner C4.

-- 1. Planner drafts
CREATE TABLE planner_drafts (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    customer_id     text NULL REFERENCES customers(id),
    session_hash    text NULL,
    branch_id       text NULL,
    currency_code   char(3) NOT NULL REFERENCES currencies(code),
    customer_side   text NOT NULL CHECK (customer_side IN ('CUSTOMER_BUY', 'CUSTOMER_SELL')),
    quantity        numeric(20,4) NULL CHECK (quantity > 0),
    budget_thb      numeric(20,2) NULL CHECK (budget_thb > 0),
    version         integer NOT NULL DEFAULT 1,
    expires_at      timestamptz NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CHECK ((customer_id IS NULL) <> (session_hash IS NULL)),
    CHECK ((quantity IS NULL) <> (budget_thb IS NULL)),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id)
);

CREATE INDEX planner_drafts_customer ON planner_drafts (customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX planner_drafts_session ON planner_drafts (session_hash) WHERE session_hash IS NOT NULL;

-- 2. Customer app settings
CREATE TABLE customer_app_settings (
    organization_id           uuid PRIMARY KEY REFERENCES organizations(id),
    planner_draft_ttl_seconds integer NULL CHECK (planner_draft_ttl_seconds > 0),
    updated_by                text NOT NULL,
    updated_at                timestamptz NOT NULL
);

-- 3. Notifications
CREATE TABLE notifications (
    id                uuid PRIMARY KEY,
    seq               bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    organization_id   uuid NOT NULL REFERENCES organizations(id),
    recipient_user_id uuid NOT NULL REFERENCES users(id),
    event_id          text NOT NULL,
    template_key      text NOT NULL,
    template_version  integer NOT NULL,
    locale            text NOT NULL CHECK (locale IN ('th', 'en', 'zh')),
    object_type       text NULL,
    object_id         text NULL,
    params            jsonb NOT NULL DEFAULT '{}',
    created_at        timestamptz NOT NULL DEFAULT now(),
    read_at           timestamptz NULL,
    UNIQUE (recipient_user_id, event_id, template_key)
);

CREATE INDEX notifications_unread ON notifications (recipient_user_id, seq) WHERE read_at IS NULL;

-- 4. Notification deliveries
CREATE TABLE notification_deliveries (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    notification_id uuid NOT NULL REFERENCES notifications(id),
    channel         text NOT NULL CHECK (channel IN ('IN_APP', 'EMAIL', 'PUSH', 'SMS')),
    status          text NOT NULL CHECK (status IN ('PENDING', 'SENT', 'FAILED', 'SKIPPED')),
    attempt_no      integer NOT NULL DEFAULT 0,
    provider_ref    text NULL,
    next_retry_at   timestamptz NULL,
    skipped_reason  text NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (notification_id, channel)
);

-- 5. Assistant conversations
CREATE TABLE assistant_conversations (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    customer_id     text NULL REFERENCES customers(id),
    session_hash    text NULL,
    locale          text NOT NULL CHECK (locale IN ('th', 'en', 'zh')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NULL,
    CHECK ((customer_id IS NULL) <> (session_hash IS NULL))
);

-- 6. Assistant messages
CREATE TABLE assistant_messages (
    id              uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id),
    conversation_id uuid NOT NULL REFERENCES assistant_conversations(id),
    role            text NOT NULL CHECK (role IN ('CUSTOMER', 'ASSISTANT')),
    redacted_text   text NOT NULL,
    parsed_intent   jsonb NULL,
    quote_id        uuid NULL REFERENCES quotes(id),
    idempotency_key text NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, idempotency_key)
);

-- Seed: one customer_app_settings row per existing organisation with 7 days (604800 seconds, owner C4)
INSERT INTO customer_app_settings (organization_id, planner_draft_ttl_seconds, updated_by, updated_at)
SELECT id, 604800, 'migration', now() FROM organizations;

-- Down comment:
-- Forward-only: rollback is a restore. Dropping tables loses drafts, notifications, delivery records, assistant conversations, and settings.
