-- pricing-formula #5495: migration §8 (design docs/design/blueprint/pricing-formula.md §8; approved as Q-F1, Q-F3 in §10).

CREATE TABLE pricing_formula_versions (
    id               uuid PRIMARY KEY,
    organization_id  uuid NOT NULL REFERENCES organizations(id),
    version          integer NOT NULL,
    buy_formula      text NOT NULL CHECK (length(buy_formula) BETWEEN 1 AND 500),
    sell_formula     text NOT NULL CHECK (length(sell_formula) BETWEEN 1 AND 500),
    valid_from       timestamptz NOT NULL,
    valid_to         timestamptz NULL,
    approved_by      text NOT NULL,
    reason           text NOT NULL,
    source_version   integer NULL,
    UNIQUE (organization_id, version)
);
CREATE UNIQUE INDEX pricing_formula_versions_current
    ON pricing_formula_versions (organization_id) WHERE valid_to IS NULL;

ALTER TABLE rate_policy_versions
    ADD COLUMN buy_formula  text NULL,
    ADD COLUMN sell_formula text NULL,
    ADD CONSTRAINT rate_policy_versions_formula_pair
        CHECK ((buy_formula IS NULL) = (sell_formula IS NULL));

ALTER TABLE rate_draft_lines
    ADD COLUMN formula_source text NULL
        CHECK (formula_source IS NULL OR formula_source IN ('BRANCH', 'ORGANISATION', 'DEFAULT')),
    ADD COLUMN formula_version_id uuid NULL REFERENCES pricing_formula_versions(id);

-- added with F-7 (FOLLOW uses the formula); approved as Q-F3 (§10)
ALTER TABLE pricing_tier_sets
    ADD COLUMN formula_source text NULL
        CHECK (formula_source IS NULL OR formula_source IN ('BRANCH', 'ORGANISATION', 'DEFAULT')),
    ADD COLUMN formula_version_id uuid NULL REFERENCES pricing_formula_versions(id);
