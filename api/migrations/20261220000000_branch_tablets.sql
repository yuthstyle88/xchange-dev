-- Branch tablets (kiosk-shared-phone §7). Additive: a new table; device_agents and every existing row are untouched.
-- Design: docs/design/blueprint/kiosk-shared-phone.md §7.6. Owner-approved (§7.1 R-T8, #4562
-- issuecomment-6330800215). No backfill. Renumber with scripts/migration-slot.sh right before merge.
CREATE TABLE branch_tablets (
    id                    uuid        PRIMARY KEY,
    branch_id             text        NOT NULL REFERENCES branches (id),
    name                  text        NOT NULL,
    created_at            timestamptz NOT NULL,
    created_by            text        NOT NULL,
    pair_code_hash        text        UNIQUE,
    pair_code_expires_at  timestamptz,
    token_hash            text        UNIQUE,
    paired_at             timestamptz,
    revoked_at            timestamptz,
    revoked_by            text,
    last_seen_at          timestamptz,
    user_agent            text,
    -- The customer session open on this tablet (user_sessions.hash), and its user. No FK: sign-out deletes the session row.
    customer_session_hash text        UNIQUE,
    customer_user_id      uuid
);
CREATE INDEX branch_tablets_branch_idx ON branch_tablets (branch_id, created_at, id);
