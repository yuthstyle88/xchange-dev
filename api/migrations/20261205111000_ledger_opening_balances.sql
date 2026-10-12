-- LG-4 #929: migration 2, the opening backfill (design `docs/design/blueprint/ledger-inventory.md` §9 item 2;
-- ticket §8, §10; owner's Q-3, 2026-10-10: the opening cost is the branch's published BUY rate).
--
-- The ledger takes over from the v2 balances it reads here and never changes:
--   - every OPEN shift: one OPENING posting, BANK → DRAWER(shift) of its `shift_balances.expected`;
--   - every branch with `ops_state.safe`: one OPENING posting, BANK → SAFE(branch) of those amounts (minor units);
--   - each posting holds one entry pair per currency (lane controller, issuecomment-6115055843);
--   - per branch and foreign currency held, one lot and one ACQUIRE movement at the published BUY rate ÷ `unit_size`
--     (8 places), and the matching `inventory_positions` row. THB is not inventory.
-- So each OPEN drawer reads its `shift_balances.expected` and each safe its `ops_state.safe`, as before the cutover.
-- A CLOSED shift is history and is not opened.
--
-- A foreign currency held with no published BUY rate at its branch stops the migration naming it: a silent zero
-- cost would be wrong. A balance below zero stops it too: the ledger holds a drawer or safe below zero only under
-- an approved request.

CREATE TEMP TABLE lg4_opening (
    branch_id text NOT NULL,
    kind      text NOT NULL,
    shift_id  text NULL,
    currency  char(3) NOT NULL,
    quantity  numeric NOT NULL
) ON COMMIT DROP;

INSERT INTO lg4_opening (branch_id, kind, shift_id, currency, quantity)
SELECT s.branch_id, 'DRAWER', s.id, b.code, b.expected
  FROM shift_balances b
  JOIN shifts s ON s.id = b.shift_id
 WHERE s.status = 'OPEN' AND b.expected <> 0
UNION ALL
SELECT o.branch_id, 'SAFE', NULL, j.key, j.value::numeric / 100
  FROM ops_state o
 CROSS JOIN LATERAL jsonb_each_text(CASE WHEN jsonb_typeof(o.safe) = 'object' THEN o.safe ELSE '{}'::jsonb END) j
 WHERE j.value::numeric <> 0;

CREATE TEMP TABLE lg4_rate ON COMMIT DROP AS
SELECT DISTINCT ON (b.id, trim(l.currency_code))
       b.id AS branch_id,
       trim(l.currency_code) AS currency,
       round(l.buy_rate / l.unit_size, 8) AS unit_cost
  FROM branches b
  JOIN published_rate_lines l ON l.publication_id = b.current_publication_id
 WHERE l.buy_rate IS NOT NULL
 ORDER BY b.id, trim(l.currency_code), (l.denomination_group = 'ALL') DESC, l.denomination_group;

DO $$
DECLARE
    offending text;
BEGIN
    SELECT string_agg(branch_id || ' ' || trim(currency), ', ' ORDER BY branch_id, currency)
      INTO offending
      FROM lg4_opening
     WHERE quantity < 0;
    IF offending IS NOT NULL THEN
        RAISE EXCEPTION 'ledger opening balances: a balance below zero cannot be opened: %', offending;
    END IF;

    SELECT string_agg(DISTINCT o.branch_id || ' ' || trim(o.currency), ', ')
      INTO offending
      FROM lg4_opening o
      LEFT JOIN lg4_rate r ON r.branch_id = o.branch_id AND r.currency = trim(o.currency)
     WHERE trim(o.currency) <> 'THB' AND r.unit_cost IS NULL;
    IF offending IS NOT NULL THEN
        RAISE EXCEPTION 'ledger opening balances: no published BUY rate for %', offending;
    END IF;
END $$;

-- One posting per OPEN shift and per safe.
CREATE TEMP TABLE lg4_posting ON COMMIT DROP AS
SELECT gen_random_uuid() AS id, p.branch_id, p.kind, p.shift_id, b.organization_id
  FROM (SELECT DISTINCT branch_id, kind, shift_id FROM lg4_opening) p
  JOIN branches b ON b.id = p.branch_id;

INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by)
SELECT id, organization_id, branch_id, 'OPENING',
       CASE kind WHEN 'DRAWER' THEN 'shift:' || shift_id ELSE 'safe:' || branch_id END,
       'opening-balance', 'POSTED', now(), 'system'
  FROM lg4_posting;

INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, created_at)
SELECT gen_random_uuid(), a.organization_id, a.branch_id, a.currency, a.kind, a.shift_id, now()
  FROM (
    SELECT DISTINCT p.organization_id, o.branch_id, o.currency, o.kind, o.shift_id
      FROM lg4_opening o
      JOIN lg4_posting p ON p.branch_id = o.branch_id AND p.kind = o.kind
                        AND p.shift_id IS NOT DISTINCT FROM o.shift_id
    UNION
    SELECT DISTINCT p.organization_id, o.branch_id, o.currency, 'BANK', NULL::text
      FROM lg4_opening o
      JOIN lg4_posting p ON p.branch_id = o.branch_id AND p.kind = o.kind
                        AND p.shift_id IS NOT DISTINCT FROM o.shift_id
  ) a
ON CONFLICT (organization_id, branch_id, currency, kind, shift_id) DO NOTHING;

-- An entry pair per currency: BANK −, the place +. A foreign entry is valued at the opening cost.
INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, valuation_thb, created_at)
SELECT p.id, a.id, o.currency, side.sign * o.quantity,
       side.sign * CASE WHEN trim(o.currency) = 'THB' THEN round(o.quantity, 2)
                        ELSE round(o.quantity * r.unit_cost, 2) END,
       now()
  FROM lg4_opening o
  JOIN lg4_posting p ON p.branch_id = o.branch_id AND p.kind = o.kind
                    AND p.shift_id IS NOT DISTINCT FROM o.shift_id
  LEFT JOIN lg4_rate r ON r.branch_id = o.branch_id AND r.currency = trim(o.currency)
 CROSS JOIN (VALUES (-1, 'BANK'), (1, 'PLACE')) AS side(sign, account)
  JOIN cash_accounts a ON a.organization_id = p.organization_id AND a.branch_id = o.branch_id
                      AND a.currency = o.currency
                      AND a.kind = CASE side.account WHEN 'BANK' THEN 'BANK' ELSE o.kind END
                      AND a.shift_id IS NOT DISTINCT FROM CASE side.account WHEN 'BANK' THEN NULL ELSE o.shift_id END;

-- Per branch and foreign currency: one lot of all it holds, acquired by its safe's posting (or else its first
-- drawer's), costed at the published BUY rate.
CREATE TEMP TABLE lg4_lot ON COMMIT DROP AS
SELECT gen_random_uuid() AS id, h.organization_id, h.branch_id, h.currency, h.quantity, r.unit_cost,
       round(h.quantity * r.unit_cost, 2) AS cost_thb, h.posting_id
  FROM (
    SELECT p.organization_id, o.branch_id, o.currency, sum(o.quantity) AS quantity,
           (array_agg(p.id ORDER BY (o.kind = 'SAFE') DESC, o.shift_id))[1] AS posting_id
      FROM lg4_opening o
      JOIN lg4_posting p ON p.branch_id = o.branch_id AND p.kind = o.kind
                        AND p.shift_id IS NOT DISTINCT FROM o.shift_id
     WHERE trim(o.currency) <> 'THB'
     GROUP BY p.organization_id, o.branch_id, o.currency
  ) h
  JOIN lg4_rate r ON r.branch_id = h.branch_id AND r.currency = trim(h.currency)
 WHERE h.quantity > 0;

INSERT INTO inventory_lots (id, organization_id, branch_id, currency, denomination_id, acquired_at, quantity, unit_cost_thb, source_posting_id)
SELECT id, organization_id, branch_id, currency, NULL, now(), quantity, unit_cost, posting_id
  FROM lg4_lot;

INSERT INTO inventory_movements (organization_id, branch_id, currency, lot_id, posting_id, transaction_line_id, movement_type, quantity_signed, cost_thb_signed, average_after, created_at)
SELECT organization_id, branch_id, currency, id, posting_id, NULL, 'ACQUIRE', quantity, cost_thb,
       round(cost_thb / quantity, 8), now()
  FROM lg4_lot;

INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version)
SELECT organization_id, branch_id, currency, quantity, cost_thb, 1
  FROM lg4_lot;

-- down:
-- DO $$
-- BEGIN
--     IF EXISTS (SELECT 1 FROM ledger_postings WHERE source_type <> 'OPENING') THEN
--         RAISE EXCEPTION 'ledger opening balances: down is refused, a posting other than OPENING exists';
--     END IF;
-- END $$;
-- DELETE FROM inventory_movements WHERE posting_id IN (SELECT id FROM ledger_postings WHERE source_type = 'OPENING');
-- DELETE FROM ledger_entries WHERE posting_id IN (SELECT id FROM ledger_postings WHERE source_type = 'OPENING');
-- DELETE FROM inventory_lots WHERE source_posting_id IN (SELECT id FROM ledger_postings WHERE source_type = 'OPENING');
-- DELETE FROM ledger_postings WHERE source_type = 'OPENING';
-- DELETE FROM inventory_positions;
