-- RV-1 (#682): reversals and receipts.kind (design docs/design/blueprint/reversal.md §3.1, §3.2, §7).
--
-- Existing rows: every receipts row is revision 1 of a sale (#551 migration 2), so it takes kind 'RECEIPT' and
-- satisfies the new CHECK. No reversals backfill: v3 has never reversed anything. An older binary never reads
-- reversals and writes receipts revision 1 only, which the default covers.

-- Precheck (§7): a receipt revision other than 1 has no kind this migration can give it.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM receipts WHERE revision <> 1) THEN
        RAISE EXCEPTION 'receipts holds a revision other than 1; reversals.sql cannot classify it';
    END IF;
END $$;

CREATE TABLE reversals (
    id                           uuid          PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id              uuid          NOT NULL REFERENCES organizations(id),
    branch_id                    text          NOT NULL,
    original_transaction_id      uuid          NOT NULL REFERENCES transactions(id),
    status                       text          NOT NULL
        CHECK (status IN ('REQUESTED', 'APPROVED', 'POSTED', 'REJECTED')),
    reason                       text          NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    -- the original's settled THB (V-1)
    amount_thb                   numeric(20,2) NOT NULL CHECK (amount_thb > 0),
    -- the original's completion date in the branch timezone (R-1)
    business_date                date          NOT NULL,
    -- NULL only while REQUESTED: AP-1 reads the row before its request exists (RG6)
    approval_request_id          uuid          NULL REFERENCES approval_requests(id),
    -- set at POSTED; RV-3 (#684) adds the foreign key to ledger_postings (RG1)
    posting_id                   uuid          NULL,
    receipt_id                   uuid          NULL REFERENCES receipts(id),
    shift_id                     text          NULL,
    requested_by                 text          NOT NULL,
    requested_at                 timestamptz   NOT NULL,
    posted_by                    text          NULL,
    posted_at                    timestamptz   NULL,
    realized_margin_reversed_thb numeric(20,2) NULL,
    cost_difference_thb          numeric(20,2) NULL,
    created_at                   timestamptz   NOT NULL DEFAULT now(),
    updated_at                   timestamptz   NOT NULL DEFAULT now(),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id),
    CHECK (status = 'REQUESTED' OR approval_request_id IS NOT NULL),
    CHECK ((status = 'POSTED') = (posting_id IS NOT NULL AND receipt_id IS NOT NULL AND posted_at IS NOT NULL))
);

-- V-5: one live reversal per transaction; a REJECTED one frees the next request.
CREATE UNIQUE INDEX one_live_reversal_per_transaction
    ON reversals (original_transaction_id) WHERE status <> 'REJECTED';

-- V-4: revision 1 is the sale's receipt; a reversal's document is a later revision.
ALTER TABLE receipts
    ADD COLUMN kind text NOT NULL DEFAULT 'RECEIPT' CHECK (kind IN ('RECEIPT', 'REVERSAL')),
    ADD CHECK ((kind = 'RECEIPT') = (revision = 1));
