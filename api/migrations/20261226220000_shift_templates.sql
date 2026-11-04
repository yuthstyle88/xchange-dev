-- Configurable shifts (docs/design/configurable-shifts.md §5).
-- Owner 2026-11-04: "ระบบให้ รองรับ กะ สามารถ config ได้ ว่ามีกี่กะ"; defaults "มาตรฐานการทำงาน 24 ชม ไม่มีพัก".

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

-- The owner's defaults for every branch (2026-11-04 ~21:4x ICT: 3 x 8 h, 24-hour operation, no breaks, cutover 06:00).
-- ShiftPlan::defaults() holds the same rows (test S4).
INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, position)
SELECT b.organization_id, b.id, d.name, d.starts_at, d.ends_at, d.position
  FROM branches b
 CROSS JOIN (VALUES
       ('A', time '06:00', time '14:00', 0),
       ('B', time '14:00', time '22:00', 1),
       ('C', time '22:00', time '06:00', 2)
     ) AS d (name, starts_at, ends_at, position);

-- A teller's shift records the template, its name at opening, and the business date.
ALTER TABLE shifts
    ADD COLUMN template_id   uuid REFERENCES shift_templates (id),
    ADD COLUMN template_name text,
    ADD COLUMN business_date date;
ALTER TABLE shifts
    ADD CONSTRAINT shifts_template_named CHECK ((template_id IS NULL) = (template_name IS NULL));

-- A shift written without a business date (one opened with no template) is dated by the local day it opened on,
-- the same rule as the backfill below. Templated shifts always write it (ShiftService::open_on_verified).
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

-- Shifts opened before this feature: the local calendar date they opened on. A pre-feature shift opened at 00:30
-- therefore keeps the calendar date, matching the calendar value_date its sales kept (owner Q6 = A: no re-dating).
UPDATE shifts s
   SET business_date = (s.opened_at AT TIME ZONE b.timezone)::date
  FROM branches b
 WHERE b.id = s.branch_id;
ALTER TABLE shifts ALTER COLUMN business_date SET NOT NULL;

-- The teller's choice is made on the OPEN count and copied to the shift on verify. Every count carries a business date
-- (design §2.3); a count written without one is dated by the local day it was counted, like the backfill.
ALTER TABLE cash_counts
    ADD COLUMN shift_template_id uuid REFERENCES shift_templates (id),
    ADD COLUMN business_date     date;
ALTER TABLE cash_counts
    ADD CONSTRAINT cash_counts_template_only_open CHECK (shift_template_id IS NULL OR kind = 'OPEN');
CREATE FUNCTION cash_counts_business_date_default() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.business_date IS NULL THEN
        SELECT (NEW.counted_at AT TIME ZONE b.timezone)::date
          INTO NEW.business_date
          FROM branches b
         WHERE b.id = NEW.branch_id;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER cash_counts_business_date_default
    BEFORE INSERT ON cash_counts
    FOR EACH ROW EXECUTE FUNCTION cash_counts_business_date_default();
UPDATE cash_counts c
   SET business_date = (c.counted_at AT TIME ZONE b.timezone)::date
  FROM branches b
 WHERE b.id = c.branch_id;
ALTER TABLE cash_counts ALTER COLUMN business_date SET NOT NULL;
