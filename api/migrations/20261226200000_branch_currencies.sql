-- docs/design/board-currencies.md §6, §7: the branch currency set. Every currency list reads this table.
-- The owner approved this migration on PR 6073 (issuecomment-6623877354).

-- 1. Extend the kiosk switch: one row per branch, currency and group.
ALTER TABLE kiosk_currencies RENAME TO branch_currencies;
ALTER TABLE branch_currencies RENAME COLUMN code TO currency_code;
ALTER TABLE branch_currencies
    ADD COLUMN group_code text NOT NULL DEFAULT 'ALL',
    ADD COLUMN display text NOT NULL DEFAULT 'HIDE' CHECK (display IN ('SHOW', 'ASK', 'HIDE')),
    ADD COLUMN sort_order integer NOT NULL DEFAULT 1000,
    ADD COLUMN updated_by text,
    ADD COLUMN updated_at timestamptz;
UPDATE branch_currencies SET display = CASE WHEN enabled THEN 'SHOW' ELSE 'HIDE' END;
ALTER TABLE branch_currencies DROP COLUMN enabled;
ALTER TABLE branch_currencies DROP CONSTRAINT kiosk_currencies_pkey;
ALTER TABLE branch_currencies ADD PRIMARY KEY (branch_id, currency_code, group_code);

-- 2. Notes may be linked to a row, and cannot repeat.
ALTER TABLE denominations ADD CONSTRAINT denominations_id_currency UNIQUE (id, currency_code);
CREATE UNIQUE INDEX denominations_note
    ON denominations (organization_id, currency_code, group_code, face_value)
    WHERE face_value IS NOT NULL;

CREATE TABLE branch_currency_notes (
    branch_id       text    NOT NULL,
    currency_code   char(3) NOT NULL,
    group_code      text    NOT NULL,
    denomination_id bigint  NOT NULL,
    PRIMARY KEY (branch_id, currency_code, group_code, denomination_id),
    FOREIGN KEY (branch_id, currency_code, group_code)
        REFERENCES branch_currencies (branch_id, currency_code, group_code),
    FOREIGN KEY (denomination_id, currency_code)
        REFERENCES denominations (id, currency_code)
);

-- 3. The notes of the 33 currencies that have none (§7, public note series, the owner chose this seed).
INSERT INTO denominations (organization_id, currency_code, face_value, group_code)
SELECT b.organization_id, v.code, v.face, 'ALL'
FROM branches b
CROSS JOIN (VALUES
    ('AED', 5), ('AED', 10), ('AED', 20), ('AED', 50), ('AED', 100), ('AED', 200),
    ('AED', 500), ('AED', 1000), ('AUD', 5), ('AUD', 10), ('AUD', 20), ('AUD', 50),
    ('AUD', 100), ('BDT', 10), ('BDT', 20), ('BDT', 50), ('BDT', 100), ('BDT', 500),
    ('BDT', 1000), ('BHD', 0.5), ('BHD', 1), ('BHD', 5), ('BHD', 10), ('BHD', 20),
    ('BND', 1), ('BND', 5), ('BND', 10), ('BND', 20), ('BND', 50), ('BND', 100),
    ('BND', 500), ('BND', 1000), ('BND', 10000), ('CAD', 5), ('CAD', 10), ('CAD', 20),
    ('CAD', 50), ('CAD', 100), ('CHF', 10), ('CHF', 20), ('CHF', 50), ('CHF', 100),
    ('CHF', 200), ('CHF', 1000), ('DKK', 50), ('DKK', 100), ('DKK', 200), ('DKK', 500),
    ('HKD', 10), ('HKD', 20), ('HKD', 50), ('HKD', 100), ('HKD', 500), ('HKD', 1000),
    ('IDR', 1000), ('IDR', 2000), ('IDR', 5000), ('IDR', 10000), ('IDR', 20000), ('IDR', 50000),
    ('IDR', 100000), ('ILS', 20), ('ILS', 50), ('ILS', 100), ('ILS', 200), ('INR', 10),
    ('INR', 20), ('INR', 50), ('INR', 100), ('INR', 200), ('INR', 500), ('JOD', 1),
    ('JOD', 5), ('JOD', 10), ('JOD', 20), ('JOD', 50), ('KWD', 0.25), ('KWD', 0.5),
    ('KWD', 1), ('KWD', 5), ('KWD', 10), ('KWD', 20), ('LAK', 5000), ('LAK', 10000),
    ('LAK', 20000), ('LAK', 50000), ('LAK', 100000), ('MMK', 500), ('MMK', 1000), ('MMK', 5000),
    ('MMK', 10000), ('MOP', 10), ('MOP', 20), ('MOP', 50), ('MOP', 100), ('MOP', 500),
    ('MOP', 1000), ('MYR', 1), ('MYR', 5), ('MYR', 10), ('MYR', 20), ('MYR', 50),
    ('MYR', 100), ('NOK', 50), ('NOK', 100), ('NOK', 200), ('NOK', 500), ('NOK', 1000),
    ('NPR', 50), ('NPR', 100), ('NPR', 500), ('NPR', 1000), ('NZD', 5), ('NZD', 10),
    ('NZD', 20), ('NZD', 50), ('NZD', 100), ('OMR', 1), ('OMR', 5), ('OMR', 10),
    ('OMR', 20), ('OMR', 50), ('PHP', 20), ('PHP', 50), ('PHP', 100), ('PHP', 200),
    ('PHP', 500), ('PHP', 1000), ('PKR', 50), ('PKR', 100), ('PKR', 500), ('PKR', 1000),
    ('PKR', 5000), ('QAR', 1), ('QAR', 5), ('QAR', 10), ('QAR', 50), ('QAR', 100),
    ('QAR', 200), ('QAR', 500), ('RUB', 10), ('RUB', 50), ('RUB', 100), ('RUB', 200),
    ('RUB', 500), ('RUB', 1000), ('RUB', 2000), ('RUB', 5000), ('SAR', 1), ('SAR', 5),
    ('SAR', 10), ('SAR', 20), ('SAR', 50), ('SAR', 100), ('SAR', 200), ('SAR', 500),
    ('SEK', 20), ('SEK', 50), ('SEK', 100), ('SEK', 200), ('SEK', 500), ('SEK', 1000),
    ('SGD', 2), ('SGD', 5), ('SGD', 10), ('SGD', 50), ('SGD', 100), ('SGD', 1000),
    ('TRY', 5), ('TRY', 10), ('TRY', 20), ('TRY', 50), ('TRY', 100), ('TRY', 200),
    ('TWD', 100), ('TWD', 200), ('TWD', 500), ('TWD', 1000), ('TWD', 2000), ('VND', 10000),
    ('VND', 20000), ('VND', 50000), ('VND', 100000), ('VND', 200000), ('VND', 500000), ('ZAR', 10),
    ('ZAR', 20), ('ZAR', 50), ('ZAR', 100), ('ZAR', 200)
    ) AS v(code, face)
WHERE b.id = 'MAIN';

-- 4. The catch-all row of each variant group, the shape denominationGroupCreate writes. KRWS and SCP have no notes yet.
INSERT INTO denominations (organization_id, currency_code, face_value, group_code)
SELECT b.organization_id, v.code, NULL, v.grp
FROM branches b
CROSS JOIN (VALUES ('MYR', 'MYRS'), ('CNY', 'CNYS'), ('KRW', 'KRWS'), ('INR', 'INRS'), ('GBP', 'SCP')) AS v(code, grp)
WHERE b.id = 'MAIN';

-- 5. MAIN's 45 rows: publication v5's four SHOW, the board's ASK rows, everything else HIDE until priced (§6).
INSERT INTO branch_currencies (branch_id, organization_id, currency_code, group_code, display, sort_order)
SELECT b.id, b.organization_id, v.code, v.grp, v.display, v.sort_order
FROM branches b
CROSS JOIN (VALUES
    ('THB', 'ALL', 'HIDE', 1),
    ('USD', 'ALL', 'SHOW', 2),
    ('EUR', 'ALL', 'SHOW', 3),
    ('GBP', 'ALL', 'HIDE', 4),
    ('JPY', 'ALL', 'SHOW', 5),
    ('CNY', 'ALL', 'SHOW', 6),
    ('AUD', 'ALL', 'HIDE', 7),
    ('CHF', 'ALL', 'HIDE', 8),
    ('NZD', 'ALL', 'ASK', 9),
    ('SGD', 'ALL', 'ASK', 10),
    ('BND', 'ALL', 'HIDE', 11),
    ('MYR', 'ALL', 'HIDE', 12),
    ('MYR', 'MYRS', 'ASK', 13),
    ('PHP', 'ALL', 'HIDE', 14),
    ('CNY', 'CNYS', 'ASK', 15),
    ('TWD', 'ALL', 'HIDE', 16),
    ('HKD', 'ALL', 'HIDE', 17),
    ('CAD', 'ALL', 'HIDE', 18),
    ('VND', 'ALL', 'ASK', 19),
    ('KRW', 'ALL', 'HIDE', 20),
    ('KRW', 'KRWS', 'ASK', 21),
    ('INR', 'ALL', 'HIDE', 22),
    ('INR', 'INRS', 'ASK', 23),
    ('AED', 'ALL', 'HIDE', 24),
    ('QAR', 'ALL', 'HIDE', 25),
    ('SAR', 'ALL', 'HIDE', 26),
    ('OMR', 'ALL', 'HIDE', 27),
    ('BHD', 'ALL', 'HIDE', 28),
    ('KWD', 'ALL', 'HIDE', 29),
    ('GBP', 'SCP', 'ASK', 30),
    ('IDR', 'ALL', 'ASK', 31),
    ('ZAR', 'ALL', 'HIDE', 32),
    ('TRY', 'ALL', 'ASK', 33),
    ('MMK', 'ALL', 'HIDE', 34),
    ('LAK', 'ALL', 'HIDE', 35),
    ('JOD', 'ALL', 'ASK', 36),
    ('PKR', 'ALL', 'HIDE', 37),
    ('NPR', 'ALL', 'HIDE', 38),
    ('BDT', 'ALL', 'HIDE', 39),
    ('ILS', 'ALL', 'HIDE', 40),
    ('MOP', 'ALL', 'HIDE', 41),
    ('RUB', 'ALL', 'HIDE', 42),
    ('DKK', 'ALL', 'ASK', 43),
    ('SEK', 'ALL', 'ASK', 44),
    ('NOK', 'ALL', 'ASK', 45)
    ) AS v(code, grp, display, sort_order)
WHERE b.id = 'MAIN';

-- 6. Link notes: a variant takes its own, the ALL row takes the rest of its currency (§7). A note is one stock either way.
INSERT INTO branch_currency_notes (branch_id, currency_code, group_code, denomination_id)
SELECT 'MAIN', d.currency_code, v.grp, d.id
FROM denominations d
JOIN branches b ON b.id = 'MAIN' AND b.organization_id = d.organization_id
JOIN (VALUES ('MYR', 'MYRS', 5::numeric), ('MYR', 'MYRS', 10), ('MYR', 'MYRS', 20),
             ('CNY', 'CNYS', 5), ('CNY', 'CNYS', 10), ('CNY', 'CNYS', 20),
             ('INR', 'INRS', 100), ('INR', 'INRS', 200)) AS v(code, grp, face)
  ON trim(d.currency_code) = v.code AND d.face_value = v.face AND d.group_code = 'ALL';

INSERT INTO branch_currency_notes (branch_id, currency_code, group_code, denomination_id)
SELECT 'MAIN', d.currency_code, 'ALL', d.id
FROM denominations d
JOIN branches b ON b.id = 'MAIN' AND b.organization_id = d.organization_id
JOIN branch_currencies r
  ON r.branch_id = 'MAIN' AND r.currency_code = d.currency_code AND r.group_code = 'ALL'
WHERE d.face_value IS NOT NULL
  AND d.group_code = 'ALL'
  AND NOT EXISTS (
      SELECT 1 FROM branch_currency_notes n
      WHERE n.branch_id = 'MAIN' AND n.denomination_id = d.id
  );
