-- KWQ slice e: per-branch queue-machine settings (design docs/design/blueprint/kiosk-shared-phone.md §4.2 D14, §4.6).
-- Owner-approved: PR 4249 issuecomment-6322650645 ("อนุมัติ"); scope: xchange#564 issuecomment-6323680134 (additive only).
-- Additive: a new table, no existing row changes, no backfill. A branch with no row reads the defaults.
-- Slot taken after slice a's 20261219220000_kiosk_requests_customer.sql (feat/kwq-a).
-- Renumber with scripts/migration-slot.sh next right before pr_open.

CREATE TABLE kiosk_settings (
    branch_id      text PRIMARY KEY REFERENCES branches (id),
    result_seconds integer NOT NULL DEFAULT 10
        CONSTRAINT kiosk_settings_result_seconds_check CHECK (result_seconds BETWEEN 3 AND 120),
    updated_by     text NULL,
    updated_at     timestamptz NULL,
    version        bigint NOT NULL DEFAULT 0
);
