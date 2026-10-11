-- SB-1 #837 (design docs/design/blueprint/org-roles-branches.md §14.5): the branch a session works in, and the
-- user's last choice. Both NULL on every existing row = "no choice yet"; the code computes the R5 branch lazily, so
-- there is no backfill. Renamed from §14.5's 20261031100000 (taken on main by kiosk_quote_id) to sort last.
ALTER TABLE users ADD COLUMN last_branch_id text NULL;
ALTER TABLE users ADD CONSTRAINT users_last_branch_fk
  FOREIGN KEY (organization_id, last_branch_id) REFERENCES branches(organization_id, id) ON DELETE SET NULL (last_branch_id);
ALTER TABLE user_sessions ADD COLUMN branch_id text NULL;
ALTER TABLE user_sessions ADD CONSTRAINT user_sessions_branch_fk
  FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id) ON DELETE SET NULL (branch_id);
