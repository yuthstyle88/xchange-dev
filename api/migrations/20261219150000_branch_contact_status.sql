-- S3 (#3406, design docs/design/blueprint/section-b-gaps.md §S3, amends org-roles-branches.md §6):
-- A04 branch contact channels and status.
-- down: ALTER TABLE branches DROP COLUMN contact_channels, DROP COLUMN status;

ALTER TABLE branches
    ADD COLUMN contact_channels jsonb NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN status text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'INACTIVE'));
