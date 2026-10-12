//! `VersionHash` (ticket #1390 §7, design §4.1, D-3).
//!
//! sha256 over rules, branches, dates, budget, caps, stack group, priority, and segment snapshot.

use sha2::{Digest, Sha256};

use super::version::CampaignVersion;

pub struct VersionHash;

impl VersionHash {
    /// Computes the sha256 hex string over rules, branches, and version terms.
    pub fn of<R: AsRef<[u8]>, B: AsRef<str>>(
        version: &CampaignVersion,
        rules: &[R],
        branches: &[B],
    ) -> String {
        let mut hasher = Sha256::new();

        // 1. Rules
        hasher.update((rules.len() as u64).to_le_bytes());
        for rule in rules {
            let r_bytes = rule.as_ref();
            hasher.update((r_bytes.len() as u64).to_le_bytes());
            hasher.update(r_bytes);
        }

        // 2. Branches (sorted for deterministic hash regardless of order)
        let mut sorted_branches: Vec<&str> = branches.iter().map(|b| b.as_ref()).collect();
        sorted_branches.sort_unstable();
        hasher.update((sorted_branches.len() as u64).to_le_bytes());
        for branch in sorted_branches {
            hasher.update((branch.len() as u64).to_le_bytes());
            hasher.update(branch.as_bytes());
        }

        // 3. Dates (starts_at, ends_at)
        let starts_str = version.terms.starts_at.to_rfc3339();
        hasher.update((starts_str.len() as u64).to_le_bytes());
        hasher.update(starts_str.as_bytes());

        let ends_str = version.terms.ends_at.to_rfc3339();
        hasher.update((ends_str.len() as u64).to_le_bytes());
        hasher.update(ends_str.as_bytes());

        // 4. Budget
        let budget_str = version.terms.budget_thb.round_dp(2).normalize().to_string();
        hasher.update((budget_str.len() as u64).to_le_bytes());
        hasher.update(budget_str.as_bytes());

        // 5. Caps (max_uses)
        match version.terms.max_uses {
            Some(m) => {
                hasher.update([1u8]);
                hasher.update(m.to_le_bytes());
            }
            None => {
                hasher.update([0u8]);
            }
        }

        // 6. Stack group
        hasher.update((version.terms.stack_group.len() as u64).to_le_bytes());
        hasher.update(version.terms.stack_group.as_bytes());

        // 7. Priority
        hasher.update(version.terms.priority.to_le_bytes());

        // 8. Segment snapshot
        match version.terms.segment_snapshot_id {
            Some(ref id) => {
                hasher.update([1u8]);
                hasher.update(id.as_bytes());
            }
            None => {
                hasher.update([0u8]);
            }
        }

        let result = hasher.finalize();
        format!("{result:x}")
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use super::*;
    use crate::domain::campaign::version::VersionTerms;

    fn sample_version() -> CampaignVersion {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 9, 0, 0).unwrap();
        CampaignVersion::draft(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            VersionTerms {
                starts_at: Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
                ends_at: Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
                budget_thb: Decimal::from_str("1000.00").unwrap(),
                max_uses: Some(100),
                stack_group: "RATE".to_owned(),
                priority: 10,
                segment_snapshot_id: Some(Uuid::new_v4()),
            },
            Uuid::new_v4(),
            now,
        )
    }

    #[test]
    fn two_campaigns_differing_in_any_one_input_get_different_hashes() {
        let base_version = sample_version();
        let base_rules = vec!["rule_1_json", "rule_2_json"];
        let base_branches = vec!["BKK", "CNX"];

        let base_hash = VersionHash::of(&base_version, &base_rules, &base_branches);
        assert_eq!(base_hash.len(), 64, "sha256 hex string has 64 chars");

        // 1. Differ in rules
        let other_rules = vec!["rule_1_json", "rule_modified_json"];
        let hash_diff_rules = VersionHash::of(&base_version, &other_rules, &base_branches);
        assert_ne!(base_hash, hash_diff_rules, "differing in rules must differ");

        // 2. Differ in branches
        let other_branches = vec!["BKK", "HKT"];
        let hash_diff_branches = VersionHash::of(&base_version, &base_rules, &other_branches);
        assert_ne!(
            base_hash, hash_diff_branches,
            "differing in branches must differ"
        );

        // 3. Differ in starts_at
        let mut v_diff_starts = base_version.clone();
        v_diff_starts.terms.starts_at = Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap();
        let hash_diff_starts = VersionHash::of(&v_diff_starts, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_starts,
            "differing in starts_at must differ"
        );

        // 4. Differ in ends_at
        let mut v_diff_ends = base_version.clone();
        v_diff_ends.terms.ends_at = Utc.with_ymd_and_hms(2026, 12, 2, 0, 0, 0).unwrap();
        let hash_diff_ends = VersionHash::of(&v_diff_ends, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_ends,
            "differing in ends_at must differ"
        );

        // 5. Differ in budget_thb
        let mut v_diff_budget = base_version.clone();
        v_diff_budget.terms.budget_thb = Decimal::from_str("2000.00").unwrap();
        let hash_diff_budget = VersionHash::of(&v_diff_budget, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_budget,
            "differing in budget_thb must differ"
        );

        // 6. Differ in max_uses (caps)
        let mut v_diff_max_uses = base_version.clone();
        v_diff_max_uses.terms.max_uses = Some(200);
        let hash_diff_max_uses = VersionHash::of(&v_diff_max_uses, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_max_uses,
            "differing in max_uses must differ"
        );

        let mut v_diff_no_uses = base_version.clone();
        v_diff_no_uses.terms.max_uses = None;
        let hash_diff_no_uses = VersionHash::of(&v_diff_no_uses, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_no_uses,
            "differing in max_uses (None) must differ"
        );

        // 7. Differ in stack_group
        let mut v_diff_stack = base_version.clone();
        v_diff_stack.terms.stack_group = "FEE".to_owned();
        let hash_diff_stack = VersionHash::of(&v_diff_stack, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_stack,
            "differing in stack_group must differ"
        );

        // 8. Differ in priority
        let mut v_diff_priority = base_version.clone();
        v_diff_priority.terms.priority = 20;
        let hash_diff_priority = VersionHash::of(&v_diff_priority, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_priority,
            "differing in priority must differ"
        );

        // 9. Differ in segment_snapshot_id
        let mut v_diff_snapshot = base_version.clone();
        v_diff_snapshot.terms.segment_snapshot_id = Some(Uuid::new_v4());
        let hash_diff_snapshot = VersionHash::of(&v_diff_snapshot, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_snapshot,
            "differing in segment_snapshot_id must differ"
        );

        let mut v_diff_no_snapshot = base_version.clone();
        v_diff_no_snapshot.terms.segment_snapshot_id = None;
        let hash_diff_no_snapshot =
            VersionHash::of(&v_diff_no_snapshot, &base_rules, &base_branches);
        assert_ne!(
            base_hash, hash_diff_no_snapshot,
            "differing in segment_snapshot_id (None) must differ"
        );
    }

    #[test]
    fn branch_order_does_not_affect_hash() {
        let version = sample_version();
        let rules = vec!["rule_1"];
        let branches_a = vec!["BKK", "CNX", "HKT"];
        let branches_b = vec!["HKT", "BKK", "CNX"];

        let hash_a = VersionHash::of(&version, &rules, &branches_a);
        let hash_b = VersionHash::of(&version, &rules, &branches_b);
        assert_eq!(
            hash_a, hash_b,
            "branch ordering must not affect version hash"
        );
    }

    #[test]
    fn budget_normalization_gives_identical_hash() {
        let mut v1 = sample_version();
        v1.terms.budget_thb = Decimal::from_str("1000").unwrap();

        let mut v2 = v1.clone();
        v2.terms.budget_thb = Decimal::from_str("1000.00").unwrap();

        let rules = vec!["rule_1"];
        let branches = vec!["BKK"];

        let hash_1 = VersionHash::of(&v1, &rules, &branches);
        let hash_2 = VersionHash::of(&v2, &rules, &branches);

        assert_eq!(
            hash_1, hash_2,
            "the same budget written as 1000 and as 1000.00 gives an identical hash"
        );
    }
}
