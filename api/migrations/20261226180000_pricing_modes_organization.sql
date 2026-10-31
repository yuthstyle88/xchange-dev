-- #5666 D1 (#5769): pricing_modes per organisation (design docs/design/org-wide-actor-branch.md §15.14).

ALTER TABLE pricing_modes
    ADD COLUMN organization_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001' REFERENCES organizations(id);
ALTER TABLE pricing_modes DROP CONSTRAINT pricing_modes_pkey;
ALTER TABLE pricing_modes ADD PRIMARY KEY (organization_id, currency_code);

-- down:
-- Fails if two organisations hold a row for the same currency; delete the other organisations' rows first.
-- ALTER TABLE pricing_modes DROP CONSTRAINT pricing_modes_pkey;
-- ALTER TABLE pricing_modes ADD PRIMARY KEY (currency_code);
-- ALTER TABLE pricing_modes DROP COLUMN organization_id;
