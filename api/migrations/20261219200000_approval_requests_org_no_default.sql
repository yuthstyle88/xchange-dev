-- #4093 (owner decision A, issuecomment-6306957726): drop the DEFAULT on approval_requests.organization_id so a writer
-- that forgets the organisation fails loudly (23502) instead of misfiling under the default organisation.

ALTER TABLE approval_requests ALTER COLUMN organization_id DROP DEFAULT;

-- down:
-- ALTER TABLE approval_requests ALTER COLUMN organization_id SET DEFAULT '00000000-0000-0000-0000-000000000001';
