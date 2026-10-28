ALTER TABLE transactions ADD COLUMN queue_ticket_id uuid NULL REFERENCES queue_tickets (id);
CREATE UNIQUE INDEX transactions_one_per_queue_ticket
    ON transactions (queue_ticket_id)
    WHERE queue_ticket_id IS NOT NULL AND status <> 'CANCELLED';
