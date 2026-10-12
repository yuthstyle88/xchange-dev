-- VF-1 (#1386): verification cases, their documents and the policy that says which documents a case needs
-- (design docs/design/blueprint/verification.md §4.1, §8 M1; Blueprint §16 VerificationCase / VerificationDocument).
--
-- Create only. Existing rows: none changes. `files`, `transaction_files` (every desk PASSPORT scan included),
-- `customer_identities` and `retention_policies` are not touched (V15, V16; owner S-4). The new tables start empty
-- except verification_policies, which gets version 1 for every existing organisation (V11). No document image is
-- stored in the database: the only bytea column is the SHA-256 checksum (Blueprint §16, private object store).
-- An older binary never reads these tables.

CREATE TABLE verification_policies (
    organization_id         uuid        NOT NULL REFERENCES organizations(id),
    version                 integer     NOT NULL CHECK (version > 0),
    accepted_document_types text[]      NOT NULL CHECK (cardinality(accepted_document_types) > 0),
    documents_required      integer     NOT NULL CHECK (documents_required > 0),
    branch_review_allowed   boolean     NOT NULL,                       -- OQ-5, V17
    published_by            uuid        NULL REFERENCES users(id),      -- NULL only for version 1 seeded below
    published_at            timestamptz NOT NULL,
    created_at              timestamptz NOT NULL,
    PRIMARY KEY (organization_id, version),
    CHECK (version = 1 OR published_by IS NOT NULL)
);

CREATE TABLE verification_cases (
    id                   uuid        PRIMARY KEY,
    organization_id      uuid        NOT NULL REFERENCES organizations(id),
    customer_id          text        NOT NULL REFERENCES customers(id),
    provider             text        NOT NULL DEFAULT 'STAFF' CHECK (provider IN ('STAFF')),   -- OQ-3: our reviewer only
    external_case_ref    text        NULL CHECK (external_case_ref IS NULL),                   -- always NULL (OQ-3)
    status               text        NOT NULL CHECK (status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'VERIFIED',
                                                                'REJECTED', 'NEEDS_UPDATE', 'EXPIRED', 'REVOKED')),
    policy_version       integer     NOT NULL,
    branch_id            text        NULL,                              -- V17: NULL = app case (organisation queue)
    submitted_at         timestamptz NULL,
    decided_at           timestamptz NULL,
    expires_at           timestamptz NULL,
    reviewed_by          uuid        NULL REFERENCES users(id),
    -- S-5 (owner 2026-10-12, design §12): six codes, no free text.
    decision_reason_code text        NULL CHECK (decision_reason_code IN ('DOCUMENT_UNREADABLE', 'DOCUMENT_EXPIRED',
                                                                          'DETAILS_MISMATCH', 'DOCUMENT_NOT_ACCEPTED',
                                                                          'SUSPECTED_FRAUD', 'OTHER')),
    version              integer     NOT NULL DEFAULT 1,                -- expectedVersion (V5)
    created_at           timestamptz NOT NULL,
    updated_at           timestamptz NULL,
    FOREIGN KEY (organization_id, policy_version) REFERENCES verification_policies (organization_id, version),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    CHECK (status NOT IN ('VERIFIED', 'REJECTED', 'NEEDS_UPDATE', 'REVOKED')
           OR (reviewed_by IS NOT NULL AND decided_at IS NOT NULL)),
    CHECK (status NOT IN ('REJECTED', 'NEEDS_UPDATE', 'REVOKED') OR decision_reason_code IS NOT NULL)
);

-- V14: one open case per customer.
CREATE UNIQUE INDEX verification_one_open_case ON verification_cases (customer_id)
    WHERE status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'NEEDS_UPDATE');

CREATE INDEX verification_queue ON verification_cases (organization_id, branch_id, submitted_at)
    WHERE status IN ('SUBMITTED', 'UNDER_REVIEW');

CREATE TABLE verification_documents (
    id                   uuid        PRIMARY KEY,
    organization_id      uuid        NOT NULL REFERENCES organizations(id),
    case_id              uuid        NOT NULL REFERENCES verification_cases(id),
    document_type        text        NOT NULL,                          -- PASSPORT or NATIONAL_ID, per the case's policy
    encrypted_object_ref text        NULL UNIQUE,                       -- the store's reference; NULL once removed or purged
    key_id               text        NULL,                              -- S-2: set when the bytes are encrypted
    checksum             bytea       NOT NULL,                          -- SHA-256 of the uploaded bytes
    mime                 text        NOT NULL CHECK (mime IN ('image/jpeg', 'image/png', 'application/pdf')),
    size_bytes           integer     NOT NULL CHECK (size_bytes BETWEEN 1 AND 15728640),   -- file_service.rs MAX_UPLOAD_BYTES
    uploaded_at          timestamptz NOT NULL,
    uploaded_by          uuid        NOT NULL REFERENCES users(id),
    retention_until      timestamptz NULL,                              -- V13; NULL = never purged (U2)
    access_class         text        NOT NULL DEFAULT 'IDENTITY_DOCUMENT' CHECK (access_class IN ('IDENTITY_DOCUMENT')),
    removed_at           timestamptz NULL,
    purged_at            timestamptz NULL,
    created_at           timestamptz NOT NULL,
    CHECK ((encrypted_object_ref IS NULL) = (removed_at IS NOT NULL OR purged_at IS NOT NULL))
);

-- §8 M1 backfill: version 1 for every existing organisation (V11; owner OQ-1, OQ-5). An organisation created later
-- has no policy row until one is published (VF-3 answers noVerificationPolicy).
INSERT INTO verification_policies (organization_id, version, accepted_document_types, documents_required,
                                   branch_review_allowed, published_by, published_at, created_at)
SELECT id, 1, ARRAY['PASSPORT', 'NATIONAL_ID'], 1, true, NULL, now(), now() FROM organizations;
