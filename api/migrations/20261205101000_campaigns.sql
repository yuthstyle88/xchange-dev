-- MK-1 (#1390): marketing campaigns M1 (design docs/design/blueprint/marketing-campaigns.md §4.1, §8).
--
-- Tables: campaigns, campaign_versions, campaign_branches, benefit_rules,
-- campaign_simulations, margin_guards.

CREATE TABLE campaigns (
    id                 uuid PRIMARY KEY,
    organization_id    uuid NOT NULL REFERENCES organizations(id),
    name               text NOT NULL,
    current_version_id uuid NULL,
    created_by         uuid NOT NULL REFERENCES users(id),
    created_at         timestamptz NOT NULL,
    UNIQUE (organization_id, id)
);

CREATE TABLE campaign_versions (
    id                  uuid PRIMARY KEY,
    organization_id     uuid NOT NULL,
    campaign_id         uuid NOT NULL,
    version             int NOT NULL CHECK (version > 0),
    status              text NOT NULL CHECK (status IN ('DRAFT','SIMULATED','PENDING_APPROVAL','APPROVED','REJECTED','PAUSED','SUPERSEDED')),
    starts_at           timestamptz NOT NULL,
    ends_at             timestamptz NOT NULL CHECK (ends_at > starts_at),
    budget_thb          numeric(20,2) NOT NULL CHECK (budget_thb >= 0),
    max_uses            int NULL CHECK (max_uses > 0),
    stack_group         text NOT NULL,
    priority            int NOT NULL,
    segment_snapshot_id uuid NULL,
    version_hash        text NULL,
    approval_id         uuid NULL REFERENCES approval_requests(id),
    simulated_at        timestamptz NULL,
    edited_at           timestamptz NOT NULL,
    created_by          uuid NOT NULL REFERENCES users(id),
    created_at          timestamptz NOT NULL,
    FOREIGN KEY (organization_id, campaign_id) REFERENCES campaigns(organization_id, id),
    UNIQUE (campaign_id, version),
    CHECK ((status IN ('PENDING_APPROVAL','APPROVED','PAUSED','SUPERSEDED','REJECTED')) = (version_hash IS NOT NULL))
);

CREATE UNIQUE INDEX one_draft_per_campaign ON campaign_versions (campaign_id) WHERE status IN ('DRAFT','SIMULATED');

ALTER TABLE campaigns
    ADD CONSTRAINT fk_campaigns_current_version
    FOREIGN KEY (current_version_id) REFERENCES campaign_versions(id);

CREATE TABLE campaign_branches (
    campaign_version_id uuid NOT NULL REFERENCES campaign_versions(id),
    branch_id           text NOT NULL REFERENCES branches(id),
    PRIMARY KEY (campaign_version_id, branch_id)
);

CREATE TABLE benefit_rules (
    id                  uuid PRIMARY KEY,
    campaign_version_id uuid NOT NULL REFERENCES campaign_versions(id),
    benefit_type        text NOT NULL,
    benefit_class       text NOT NULL CHECK (benefit_class IN ('PRICE','REWARD')),
    eligibility         jsonb NOT NULL,
    calculation         jsonb NOT NULL,
    caps                jsonb NOT NULL,
    requires_coupon     boolean NOT NULL DEFAULT false,
    rule_no             int NOT NULL,
    UNIQUE (campaign_version_id, rule_no)
);

CREATE TABLE campaign_simulations (
    id                  uuid PRIMARY KEY,
    campaign_version_id uuid NOT NULL REFERENCES campaign_versions(id),
    input               jsonb NOT NULL,
    result              jsonb NOT NULL,
    cost_source         text NOT NULL CHECK (cost_source IN ('LEDGER','UNAVAILABLE')),
    simulated_by        uuid NOT NULL REFERENCES users(id),
    simulated_at        timestamptz NOT NULL
);

CREATE TABLE margin_guards (
    organization_id     uuid PRIMARY KEY REFERENCES organizations(id),
    max_spread_share    numeric(5,4) NULL CHECK (max_spread_share BETWEEN 0 AND 1),
    set_by              uuid NULL REFERENCES users(id),
    set_at              timestamptz NULL
);
