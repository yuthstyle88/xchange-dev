-- RV-3a (#1495): foreign key constraint from reversals.posting_id to ledger_postings(id).

ALTER TABLE reversals
    ADD CONSTRAINT reversals_posting_id_fkey
    FOREIGN KEY (posting_id) REFERENCES ledger_postings(id);

-- down:
-- ALTER TABLE reversals DROP CONSTRAINT reversals_posting_id_fkey;
