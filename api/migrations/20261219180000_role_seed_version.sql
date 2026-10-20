-- RBAC §11 (#3976): track the seed version per built-in group so permissions added after M2 reach existing databases.
-- Count after boot: SELECT role_code FROM role_permission_grants WHERE permission = 'MarketRatesRefresh';

ALTER TABLE roles
    ADD COLUMN seed_version integer NOT NULL DEFAULT 0;

UPDATE roles
   SET seed_version = 1
 WHERE built_in;

-- down:
-- ALTER TABLE roles
--     DROP COLUMN seed_version;
