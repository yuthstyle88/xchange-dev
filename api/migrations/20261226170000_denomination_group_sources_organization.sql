-- #5666 D2 (#5760): denomination_group_sources per organisation (design docs/design/org-wide-actor-branch.md §15.15).

ALTER TABLE denomination_group_sources
    ADD COLUMN organization_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001' REFERENCES organizations(id);
ALTER TABLE denomination_group_sources DROP CONSTRAINT denomination_group_sources_pkey;
ALTER TABLE denomination_group_sources ADD PRIMARY KEY (organization_id, currency_code, group_code, provider);

-- down:
-- Fails if two organisations map the same (currency_code, group_code, provider); delete the other organisations' rows first.
-- ALTER TABLE denomination_group_sources DROP CONSTRAINT denomination_group_sources_pkey;
-- ALTER TABLE denomination_group_sources ADD PRIMARY KEY (currency_code, group_code, provider);
-- ALTER TABLE denomination_group_sources DROP COLUMN organization_id;
