-- S7 (#3410): at most one active walk-in queue ticket per signed-in customer, branch, and day.
DO $$
BEGIN
    IF to_regclass('queue_tickets') IS NOT NULL THEN
        CREATE UNIQUE INDEX IF NOT EXISTS queue_tickets_one_active_app_ticket
            ON queue_tickets (branch_id, service_date, customer_id)
            WHERE customer_id IS NOT NULL
              AND service_type = 'WALK_IN'
              AND status IN ('WAITING', 'CALLED', 'SERVING');
    END IF;
END $$;
