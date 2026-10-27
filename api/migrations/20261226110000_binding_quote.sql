-- BQ-0 (#5231, design docs/design/blueprint/binding-quote.md §3, D-1, D-10):
-- admit BINDING quotes and add exchange_settings.binding_quote_ttl_seconds.

DO $$
DECLARE
    con text;
    n   integer;
BEGIN
    SELECT count(*), max(c.conname::text)
    INTO n, con
    FROM pg_constraint c
    WHERE c.conrelid = 'quotes'::regclass
      AND c.contype = 'c'
      AND pg_get_constraintdef(c.oid) LIKE '%INDICATIVE%'
      AND pg_get_constraintdef(c.oid) NOT LIKE '%BINDING%';

    IF n <> 1 THEN
        RAISE EXCEPTION 'expected 1 INDICATIVE-only CHECK on quotes, found %', n;
    END IF;

    EXECUTE format('ALTER TABLE quotes DROP CONSTRAINT %I', con);
END $$;

ALTER TABLE exchange_settings
    ADD COLUMN binding_quote_ttl_seconds integer NULL CHECK (binding_quote_ttl_seconds > 0);
