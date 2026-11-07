-- XC Counter (docs/design/xc-counter-app.md §5, §8.1). Approved by the owner on xchange#564 / PR 6361:
-- "อนุมัติ + กฎ A" (2026-11-06 09:29 ICT) and "อนุมัติ" for §5.4's column (09:59 ICT). Deploy is a separate order.
-- Forward-only and additive, apart from dropping one index. No backfill: every new setting starts NULL ("not set"),
-- and NULL is the safe value (§5.2). The drawer agent paired on xc-v3 today therefore claims no drawer job until
-- the owner turns `cash_drawer` on for it.

-- §5.1 Several counters per branch.
DROP INDEX device_agents_one_active_per_branch_idx;
ALTER TABLE device_agents ADD COLUMN counter_name text NOT NULL DEFAULT '';

-- §5.2 Per-counter config, owner-set. NULL = not set: cash_drawer off, scanner off, kiosk_lock on.
ALTER TABLE device_agents
    ADD COLUMN cash_drawer boolean,
    ADD COLUMN scanner     boolean,
    ADD COLUMN kiosk_lock  boolean;

-- §5.3 A drawer job targets a counter; NULL = any active counter of the branch whose cash_drawer is on.
ALTER TABLE device_jobs ADD COLUMN target_agent_id uuid REFERENCES device_agents (id);

-- §5.4 The counter a staff session is bound to, through the app's signed nonce. Cleared on sign-out (the row goes),
-- on switching branch and on revoking the counter.
ALTER TABLE user_sessions ADD COLUMN counter_agent_id uuid REFERENCES device_agents (id);
