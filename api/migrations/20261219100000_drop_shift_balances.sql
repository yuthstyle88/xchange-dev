-- LG-6.m (#1047, design docs/design/blueprint/ledger-inventory.md §9 migration 3, §6.1):
-- Precheck every organisation with LG-7's five reconciliation queries; if clean, drop the dead
-- stores `stock_movements`, `shift_balances` and `ops_state.safe`.

DO $$
DECLARE
    org_id uuid;
    check_id int;
    q text;
    failed boolean;
    queries text[] := ARRAY[
        $q1$SELECT e.posting_id, e.currency, sum(e.quantity_signed)
FROM ledger_entries e
JOIN ledger_postings p ON p.id = e.posting_id
WHERE p.organization_id = $1::uuid
  AND ($2::text IS NULL OR p.branch_id = $2)
GROUP BY e.posting_id, e.currency
HAVING sum(e.quantity_signed) <> 0
ORDER BY e.posting_id, e.currency
LIMIT 101$q1$,
        $q2$SELECT
    p.branch_id,
    p.currency,
    p.quantity,
    COALESCE(SUM(m.quantity_signed), 0) AS sum_qty,
    p.cost_thb,
    COALESCE(SUM(m.cost_thb_signed), 0) AS sum_cost
FROM inventory_positions p
LEFT JOIN inventory_movements m
    ON m.organization_id = p.organization_id
   AND m.branch_id = p.branch_id
   AND m.currency = p.currency
WHERE p.organization_id = $1::uuid
  AND ($2::text IS NULL OR p.branch_id = $2::text)
GROUP BY p.organization_id, p.branch_id, p.currency, p.quantity, p.cost_thb
HAVING p.quantity <> COALESCE(SUM(m.quantity_signed), 0)
    OR p.cost_thb <> COALESCE(SUM(m.cost_thb_signed), 0)
ORDER BY p.branch_id, p.currency
LIMIT 101$q2$,
        $q3$WITH places AS (
    SELECT
        a.branch_id,
        a.currency,
        COALESCE(SUM(e.quantity_signed), 0) AS quantity
    FROM cash_accounts a
    JOIN ledger_entries e ON e.account_id = a.id
    WHERE a.organization_id = $1::uuid
      AND a.branch_id IS NOT NULL
      AND ($2::text IS NULL OR a.branch_id = $2)
      AND a.kind IN ('DRAWER', 'SAFE', 'IN_TRANSIT')
      AND a.currency <> 'THB'
    GROUP BY a.branch_id, a.currency
),
positions AS (
    SELECT
        pos.branch_id,
        pos.currency,
        pos.quantity
    FROM inventory_positions pos
    WHERE pos.organization_id = $1::uuid
      AND ($2::text IS NULL OR pos.branch_id = $2)
      AND pos.currency <> 'THB'
)
SELECT
    COALESCE(pl.branch_id, pos.branch_id) || ':' || TRIM(COALESCE(pl.currency, pos.currency)) AS key,
    'places ' || COALESCE(pl.quantity, 0)::text || ' vs position ' || COALESCE(pos.quantity, 0)::text AS detail
FROM places pl
FULL OUTER JOIN positions pos
    ON pl.branch_id = pos.branch_id
   AND pl.currency = pos.currency
WHERE COALESCE(pl.quantity, 0) <> COALESCE(pos.quantity, 0)
ORDER BY key
LIMIT 101$q3$,
        $q4$SELECT key, detail FROM (
    SELECT
        t.branch_id || ':' || t.value_date::text AS key,
        'margin ' || coalesce(sum(l.realized_margin_thb), 0)::text || ' vs sales ' || sum(l.thb_amount)::text || ' - cost ' || sum(l.cost_basis_thb)::text AS detail
    FROM transaction_lines l
    JOIN transactions t ON t.id = l.transaction_id
    WHERE t.organization_id = $1::uuid
      AND ($2::text IS NULL OR t.branch_id = $2)
      AND ($3::date IS NULL OR t.value_date BETWEEN $3::date AND $4::date)
      AND l.shop_side = 'SELL'
      AND l.cost_basis_thb IS NOT NULL
      AND t.status = 'COMPLETED'
    GROUP BY t.branch_id, t.value_date
    HAVING coalesce(sum(l.realized_margin_thb), 0) <> sum(l.thb_amount) - sum(l.cost_basis_thb)
    UNION ALL
    SELECT
        l.id::text AS key,
        'uncosted' AS detail
    FROM transaction_lines l
    JOIN transactions t ON t.id = l.transaction_id
    WHERE t.organization_id = $1::uuid
      AND ($2::text IS NULL OR t.branch_id = $2)
      AND ($3::date IS NULL OR t.value_date BETWEEN $3::date AND $4::date)
      AND l.shop_side = 'SELL'
      AND l.cost_basis_thb IS NULL
      AND t.status = 'COMPLETED'
) offenders
ORDER BY key
LIMIT 101$q4$,
        $q5$SELECT p.id::text AS posting_id, sum(e.valuation_thb)::text AS detail
FROM ledger_entries e
JOIN ledger_postings p ON p.id = e.posting_id
WHERE p.organization_id = $1::uuid
  AND ($2::text IS NULL OR p.branch_id = $2)
GROUP BY p.id
HAVING sum(e.valuation_thb) <> 0
ORDER BY p.id
LIMIT 101$q5$
    ];
BEGIN
    FOR org_id IN SELECT id FROM organizations ORDER BY id LOOP
        FOR check_id IN 1..array_length(queries, 1) LOOP
            q := queries[check_id];
            EXECUTE 'SELECT EXISTS (' || q || ')'
                INTO failed
                USING org_id::text, NULL::text, NULL::date, NULL::date;
            IF failed THEN
                RAISE EXCEPTION 'reconcile check % failed', check_id;
            END IF;
        END LOOP;
    END LOOP;
END $$;

DROP TABLE stock_movements;
DROP TABLE shift_balances;
ALTER TABLE ops_state DROP COLUMN safe;

-- down:
-- CREATE TABLE shift_balances (
--     shift_id        text          NOT NULL REFERENCES shifts (id),
--     code            char(3)       NOT NULL,
--     opening         numeric(20,2) NOT NULL DEFAULT 0 CHECK (opening >= 0),
--     expected        numeric(20,2) NOT NULL DEFAULT 0 CHECK (expected >= 0),
--     counted         numeric(20,2) CHECK (counted >= 0),
--     difference      numeric(20,2),
--     organization_id uuid          NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001' REFERENCES organizations (id),
--     PRIMARY KEY (shift_id, code)
-- );
-- CREATE TABLE stock_movements (
--     id              text          PRIMARY KEY,
--     seq             bigint        GENERATED ALWAYS AS IDENTITY UNIQUE,
--     branch_id       text          NOT NULL REFERENCES branches (id),
--     shift_id        text          NOT NULL REFERENCES shifts (id),
--     code            char(3)       NOT NULL,
--     direction       text          NOT NULL CHECK (direction IN ('IN', 'OUT')),
--     amount          numeric(20,2) NOT NULL CHECK (amount > 0),
--     reason          text          NOT NULL,
--     request_id      text          NOT NULL,
--     at              timestamptz   NOT NULL,
--     by              text          NOT NULL,
--     organization_id uuid          NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001',
--     UNIQUE (branch_id, request_id),
--     FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)
-- );
-- CREATE INDEX stock_movements_shift ON stock_movements (shift_id);
-- ALTER TABLE ops_state ADD COLUMN safe jsonb;
