-- RBAC §11 M2 (#3292): permission groups on roles, role_permission_grants, role_grant_version (design docs/design/blueprint/user-management-rbac.md §11.5).
-- Count before deploy: SELECT count(*) FROM roles; -- expect 9
-- Who the seed's changes reach: SELECT role_code, count(*) FROM user_roles GROUP BY 1;

ALTER TABLE roles
    ADD COLUMN name text,
    ADD COLUMN base_role text REFERENCES roles(code),
    ADD COLUMN built_in boolean NOT NULL DEFAULT false;

UPDATE roles
   SET name = code,
       base_role = code,
       built_in = true;

ALTER TABLE roles
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN base_role SET NOT NULL;

CREATE TABLE role_permission_grants (
    role_code text NOT NULL REFERENCES roles(code) ON DELETE CASCADE,
    permission text NOT NULL,
    PRIMARY KEY (role_code, permission)
);

CREATE TABLE role_grant_version (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    version bigint NOT NULL
);

INSERT INTO role_grant_version (version) VALUES (0);

-- down:
-- DO $$
-- BEGIN
--     IF EXISTS (
--         SELECT 1 FROM user_roles ur
--         JOIN roles r ON r.code = ur.role_code
--         WHERE NOT r.built_in
--     ) THEN
--         RAISE EXCEPTION 'role_groups down: user_roles names a custom group';
--     END IF;
-- END
-- $$;
-- DELETE FROM roles WHERE NOT built_in;
-- DROP TABLE role_grant_version;
-- DROP TABLE role_permission_grants;
-- ALTER TABLE roles
--     DROP COLUMN built_in,
--     DROP COLUMN base_role,
--     DROP COLUMN name;
