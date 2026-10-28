-- BK-15 #5392: migration M3, booking_slots.closed_by (design docs/design/blueprint/booking-slots.md §4.1, §8 M3).

ALTER TABLE booking_slots
    ADD COLUMN closed_by text NULL,
    ADD CONSTRAINT booking_slots_closed_by_check CHECK (closed_by IN ('SLOT','CLOSURE','RULE')),
    ADD CONSTRAINT booking_slots_closed_by_status_check CHECK (status = 'CLOSED' OR closed_by IS NULL);

-- down:
-- ALTER TABLE booking_slots
--     DROP CONSTRAINT booking_slots_closed_by_status_check,
--     DROP CONSTRAINT booking_slots_closed_by_check,
--     DROP COLUMN closed_by;
