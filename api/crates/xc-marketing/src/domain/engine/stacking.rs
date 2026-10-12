use std::collections::HashMap;

use super::active_rules::ActiveRule;
use crate::domain::benefit::rule::BenefitClass;

pub struct Stacking;

impl Stacking {
    /// Selects the winning rules according to stacking rules:
    /// - Within each stack group, highest priority rule wins (ties: older version id wins).
    /// - Across groups, at most one PRICE and at most one REWARD benefit per line.
    pub fn resolve(eligible_rules: &[ActiveRule]) -> Vec<&ActiveRule> {
        if eligible_rules.is_empty() {
            return Vec::new();
        }

        // Helper comparison: returns true if `a` beats `b`.
        // Order: priority DESC, version_created_at ASC, version_id ASC, rule.id ASC.
        let is_better = |a: &ActiveRule, b: &ActiveRule| -> bool {
            if a.priority != b.priority {
                return a.priority > b.priority;
            }
            if a.version_created_at != b.version_created_at {
                return a.version_created_at < b.version_created_at;
            }
            if a.version_id != b.version_id {
                return a.version_id < b.version_id;
            }
            a.rule.id < b.rule.id
        };

        // 1. Group by stack_group and pick the best in each group
        let mut group_winners: HashMap<&str, &ActiveRule> = HashMap::new();
        for rule in eligible_rules {
            group_winners
                .entry(&rule.stack_group)
                .and_modify(|existing| {
                    if is_better(rule, existing) {
                        *existing = rule;
                    }
                })
                .or_insert(rule);
        }

        // 2. Across all group winners, pick at most one PRICE and at most one REWARD
        let mut best_price: Option<&ActiveRule> = None;
        let mut best_reward: Option<&ActiveRule> = None;

        for &rule in group_winners.values() {
            match rule.rule.benefit_class {
                BenefitClass::Price => {
                    best_price = match best_price {
                        Some(existing) if !is_better(rule, existing) => Some(existing),
                        _ => Some(rule),
                    };
                }
                BenefitClass::Reward => {
                    best_reward = match best_reward {
                        Some(existing) if !is_better(rule, existing) => Some(existing),
                        _ => Some(rule),
                    };
                }
            }
        }

        let mut winners = Vec::new();
        if let Some(price) = best_price {
            winners.push(price);
        }
        if let Some(reward) = best_reward {
            winners.push(reward);
        }
        winners
    }
}
