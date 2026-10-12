//! `CampaignVersion`: the stored lifecycle and the status derived from the clock (D-2, D-3, design §4.2).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::error::CampaignError;
use super::version_hash::VersionHash;

/// The stored statuses, exactly these seven (D-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredStatus {
    Draft,
    Simulated,
    PendingApproval,
    Approved,
    Rejected,
    Paused,
    Superseded,
}

impl StoredStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StoredStatus::Draft => "DRAFT",
            StoredStatus::Simulated => "SIMULATED",
            StoredStatus::PendingApproval => "PENDING_APPROVAL",
            StoredStatus::Approved => "APPROVED",
            StoredStatus::Rejected => "REJECTED",
            StoredStatus::Paused => "PAUSED",
            StoredStatus::Superseded => "SUPERSEDED",
        }
    }
}

/// Derived from the clock for an APPROVED version only; never stored (D-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedStatus {
    /// now < `starts_at`
    Scheduled,
    /// `starts_at` ≤ now < `ends_at`
    Active,
    /// now ≥ `ends_at`
    Expired,
}

/// What a draft save changes on the version row (design §4.5 `campaignVersionSave`).
#[derive(Debug, Clone, PartialEq)]
pub struct VersionTerms {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub budget_thb: Decimal,
    pub max_uses: Option<i32>,
    pub stack_group: String,
    pub priority: i32,
    pub segment_snapshot_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CampaignVersion {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub campaign_id: Uuid,
    pub version: i32,
    pub status: StoredStatus,
    pub terms: VersionTerms,
    /// sha256 hex, set at submit (D-3).
    pub version_hash: Option<String>,
    pub approval_id: Option<Uuid>,
    pub simulated_at: Option<DateTime<Utc>>,
    pub edited_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

impl CampaignVersion {
    /// A new DRAFT version `version` of `campaign_id`.
    pub fn draft(
        id: Uuid,
        organization_id: Uuid,
        campaign_id: Uuid,
        version: i32,
        terms: VersionTerms,
        created_by: Uuid,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            organization_id,
            campaign_id,
            version,
            status: StoredStatus::Draft,
            terms,
            version_hash: None,
            approval_id: None,
            simulated_at: None,
            edited_at: now,
            created_by,
            created_at: now,
        }
    }

    /// Edits the version in place: DRAFT stays DRAFT, SIMULATED returns to DRAFT; every other status
    /// refuses with `StaleVersion` and leaves the version unchanged (D-3).
    pub fn edit(&mut self, terms: VersionTerms, now: DateTime<Utc>) -> Result<(), CampaignError> {
        match self.status {
            StoredStatus::Draft | StoredStatus::Simulated => {
                self.terms = terms;
                self.status = StoredStatus::Draft;
                self.edited_at = now;
                Ok(())
            }
            StoredStatus::PendingApproval
            | StoredStatus::Approved
            | StoredStatus::Rejected
            | StoredStatus::Paused
            | StoredStatus::Superseded => Err(CampaignError::StaleVersion),
        }
    }

    /// SCHEDULED / ACTIVE / EXPIRED for an APPROVED version; `None` for every other status (D-2).
    pub fn derived_status(&self, now: DateTime<Utc>) -> Option<DerivedStatus> {
        if self.status != StoredStatus::Approved {
            return None;
        }
        if now < self.terms.starts_at {
            Some(DerivedStatus::Scheduled)
        } else if now < self.terms.ends_at {
            Some(DerivedStatus::Active)
        } else {
            Some(DerivedStatus::Expired)
        }
    }

    /// Submits a SIMULATED version for approval (design §4.2, §4.4, ticket #1390 §7).
    ///
    /// Allowed only from `SIMULATED`. From any other status it returns `CampaignError::StaleVersion`
    /// and leaves the version unchanged.
    /// Sets status to `PendingApproval` and sets `version_hash = Some(VersionHash::of(self, rules, branches))`.
    pub fn submit<R: AsRef<[u8]>, B: AsRef<str>>(
        &mut self,
        rules: &[R],
        branches: &[B],
        _now: DateTime<Utc>,
    ) -> Result<(), CampaignError> {
        if self.status != StoredStatus::Simulated {
            return Err(CampaignError::StaleVersion);
        }
        self.status = StoredStatus::PendingApproval;
        self.version_hash = Some(VersionHash::of(self, rules, branches));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::TimeZone;

    use super::*;

    fn sample_simulated_version() -> CampaignVersion {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 9, 0, 0).unwrap();
        let mut v = CampaignVersion::draft(
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
        );
        v.status = StoredStatus::Simulated;
        v.simulated_at = Some(Utc.with_ymd_and_hms(2026, 10, 20, 10, 0, 0).unwrap());
        v
    }

    #[test]
    fn simulated_becomes_pending_approval_with_hash_set() {
        let mut v = sample_simulated_version();
        let rules = vec!["rule_1", "rule_2"];
        let branches = vec!["BKK", "CNX"];
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 11, 0, 0).unwrap();

        let res = v.submit(&rules, &branches, now);
        assert!(res.is_ok(), "SIMULATED submits successfully");
        assert_eq!(
            v.status,
            StoredStatus::PendingApproval,
            "status moves to PENDING_APPROVAL"
        );
        let hash = v.version_hash.as_ref().expect("version_hash is set");
        assert_eq!(hash.len(), 64, "version_hash is a 64-hex sha256 string");
        assert_eq!(
            *hash,
            VersionHash::of(&v, &rules, &branches),
            "version_hash matches VersionHash::of"
        );
    }

    #[test]
    fn every_status_other_than_simulated_refuses_submit_with_stale_version() {
        let rules = vec!["rule_1"];
        let branches = vec!["BKK"];
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 11, 0, 0).unwrap();

        for status in [
            StoredStatus::Draft,
            StoredStatus::PendingApproval,
            StoredStatus::Approved,
            StoredStatus::Rejected,
            StoredStatus::Paused,
            StoredStatus::Superseded,
        ] {
            let mut v = sample_simulated_version();
            v.status = status;
            if !matches!(status, StoredStatus::Draft | StoredStatus::Simulated) {
                v.version_hash = Some("some_existing_hash".to_owned());
            } else {
                v.version_hash = None;
            }
            let before = v.clone();

            let res = v.submit(&rules, &branches, now);
            assert_eq!(
                res,
                Err(CampaignError::StaleVersion),
                "{status:?} must be refused with staleVersion on submit"
            );
            assert_eq!(
                v, before,
                "{status:?} must remain unchanged when submit is refused"
            );
        }
    }
}
