-- OR-branch-org #5757 (#5665 part 2, D3 + D4): per-organisation branch name uniqueness.

DROP INDEX branches_name_lower_key;
CREATE UNIQUE INDEX branches_organization_id_name_lower_key ON branches (organization_id, lower(name));

-- down:
-- DROP INDEX branches_organization_id_name_lower_key;
-- CREATE UNIQUE INDEX branches_name_lower_key ON branches (lower(name));
