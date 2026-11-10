-- XC Counter kiosk exit (docs/design/xc-counter-app.md §8.8, xchange#6569). Owner decisions of 2026-11-09 08:09 ICT
-- ("อนุมัติ"): this table for one-time use, `CounterUnlock` for manager and owner only, a 60 s lifetime.
-- Additive: a new table, nothing else changes. Deploy is a separate order.
--
-- One row per code a manager issues at the desk. The code itself is never stored, only its hash (the pairing code's
-- hash, `DeviceSecret::code_hash`). The row is also the audit record: who issued it, for which counter, when, and
-- whether and when that counter used it.
CREATE TABLE counter_unlock_tokens (
    id          uuid        PRIMARY KEY,
    agent_id    uuid        NOT NULL REFERENCES device_agents (id),
    code_hash   text        NOT NULL UNIQUE,
    issued_by   uuid        NOT NULL REFERENCES users (id),
    issued_at   timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL,
    redeemed_at timestamptz
);
