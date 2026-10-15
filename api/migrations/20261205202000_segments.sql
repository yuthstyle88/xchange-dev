-- MK-6 (#1404): marketing segments M3 (design docs/design/blueprint/marketing-campaigns.md §4.1, §8, ruling Q3).
--
-- Tables: segment_definitions, segment_snapshots, segment_snapshot_members
-- FK: campaign_versions.segment_snapshot_id -> segment_snapshots(id)

CREATE TABLE segment_definitions (
    id                 uuid PRIMARY KEY,
    organization_id    uuid NOT NULL REFERENCES organizations(id),
    name               text NOT NULL,
    version            int NOT NULL CHECK (version > 0),
    rule               jsonb NOT NULL,
    created_by         uuid NOT NULL,
    created_at         timestamptz NOT NULL,
    UNIQUE (organization_id, name, version)
);

CREATE TABLE segment_snapshots (
    id                 uuid PRIMARY KEY,
    organization_id    uuid NOT NULL REFERENCES organizations(id),
    definition_id      uuid NOT NULL REFERENCES segment_definitions(id),
    as_of              timestamptz NOT NULL,
    member_count       int NOT NULL CHECK (member_count >= 0),
    taken_by           uuid NOT NULL,
    created_at         timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE segment_snapshot_members (
    snapshot_id        uuid NOT NULL REFERENCES segment_snapshots(id) ON DELETE CASCADE,
    customer_id        text NOT NULL REFERENCES customers(id),
    metrics            jsonb NOT NULL,
    PRIMARY KEY (snapshot_id, customer_id)
);

ALTER TABLE campaign_versions
    ADD CONSTRAINT fk_campaign_versions_segment_snapshot
    FOREIGN KEY (segment_snapshot_id) REFERENCES segment_snapshots(id);
