-- Configurable shifts (docs/design/configurable-shifts.md §5).
-- Owner 2026-11-04: "ระบบให้ รองรับ กะ สามารถ config ได้ ว่ามีกี่กะ", defaults A/B/C ("default ตามนี้").

CREATE TABLE shift_templates (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid        NOT NULL,
    branch_id       text        NOT NULL,
    name            text        NOT NULL CHECK (btrim(name) <> '' AND char_length(name) <= 40),
    starts_at       time        NOT NULL,
    ends_at         time        NOT NULL,
    break_starts_at time,
    break_ends_at   time,
    position        smallint    NOT NULL CHECK (position >= 0),
    retired_at      timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    CHECK (starts_at <> ends_at),
    CHECK ((break_starts_at IS NULL) = (break_ends_at IS NULL))
);
CREATE UNIQUE INDEX shift_templates_live_name
    ON shift_templates (branch_id, lower(btrim(name))) WHERE retired_at IS NULL;
CREATE INDEX shift_templates_live_branch
    ON shift_templates (branch_id, position) WHERE retired_at IS NULL;

-- The owner's defaults for every existing branch. ShiftPlan::defaults() holds the same rows (test S4).
INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, break_starts_at, break_ends_at, position)
SELECT b.organization_id, b.id, d.name, d.starts_at, d.ends_at, d.break_starts_at, d.break_ends_at, d.position
  FROM branches b
 CROSS JOIN (VALUES
       ('A', time '10:00', time '19:00', time '15:00', time '16:00', 0),
       ('B', time '15:00', time '00:00', time '18:00', time '19:00', 1),
       ('C', time '19:00', time '04:00', time '23:00', time '00:00', 2)
     ) AS d (name, starts_at, ends_at, break_starts_at, break_ends_at, position)
 WHERE b.organization_id IS NOT NULL;

-- A teller's shift records the template, its name at opening, and the business date.
ALTER TABLE shifts
    ADD COLUMN template_id   uuid REFERENCES shift_templates (id),
    ADD COLUMN template_name text,
    ADD COLUMN business_date date;
ALTER TABLE shifts
    ADD CONSTRAINT shifts_template_named CHECK ((template_id IS NULL) = (template_name IS NULL));

-- A shift written without a business date (one opened with no template) is dated by the local day it opened on.
-- Templated shifts always write it (ShiftService::open_on_verified).
CREATE FUNCTION shifts_business_date_default() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.business_date IS NULL THEN
        SELECT (NEW.opened_at AT TIME ZONE b.timezone)::date
          INTO NEW.business_date
          FROM branches b
         WHERE b.id = NEW.branch_id;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER shifts_business_date_default
    BEFORE INSERT ON shifts
    FOR EACH ROW EXECUTE FUNCTION shifts_business_date_default();

UPDATE shifts s
   SET business_date = (s.opened_at AT TIME ZONE b.timezone)::date
  FROM branches b
 WHERE b.id = s.branch_id;
ALTER TABLE shifts ALTER COLUMN business_date SET NOT NULL;

-- The teller's choice is made on the OPEN count and copied to the shift on verify.
ALTER TABLE cash_counts
    ADD COLUMN shift_template_id uuid REFERENCES shift_templates (id),
    ADD COLUMN business_date     date;
ALTER TABLE cash_counts
    ADD CONSTRAINT cash_counts_template_dated CHECK ((shift_template_id IS NULL) = (business_date IS NULL)),
    ADD CONSTRAINT cash_counts_template_only_open CHECK (shift_template_id IS NULL OR kind = 'OPEN');
