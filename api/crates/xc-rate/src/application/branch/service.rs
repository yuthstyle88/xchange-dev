//! Branch application service.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::branch::branch_hours::BranchHours;
use crate::domain::branch::commands::create_branch::CreateBranch;
use crate::domain::branch::contact_channel::{BranchStatus, ContactChannel};
use crate::domain::branch::entities::branch::Branch;
use crate::domain::branch::repositories::branch_repository::BranchRepository;
use crate::domain::validation::{
    ABANDONED, NAME_TAKEN, NO_SUCH_BRANCH, REQUEST_CHANGED, TOO_MANY_BRANCHES, cut_to_millis,
    validate_request_id, validate_text,
};
use crate::infrastructure::branch::branch_repository::PgBranchRepository;

/// A branch's profile (OR-1's columns), saved by an ADMIN through `branchSave` (OR-5a #945,
/// design `org-roles-branches.md` §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProfile {
    /// Unique within the branch's organisation.
    pub code: String,
    pub address: Option<String>,
    /// An IANA zone (`chrono-tz`).
    pub timezone: String,
    /// The shape `BranchHours::new` accepts.
    pub opening_hours: Option<serde_json::Value>,
    pub booking_enabled: bool,
}

#[derive(Clone)]
pub struct BranchService {
    branch_tx: BranchTx,
    repo: PgBranchRepository,
}

impl BranchService {
    pub fn new(branch_tx: BranchTx) -> Self {
        let repo = PgBranchRepository::new(branch_tx.pool().clone());
        Self { branch_tx, repo }
    }

    pub async fn list(&self) -> AppResult<Vec<Branch>> {
        self.repo.list().await
    }

    pub async fn require(&self, id: &BranchId) -> AppResult<Branch> {
        match self.repo.find_by_id(id).await? {
            Some(branch) => Ok(branch),
            None => Err(AppError::Domain(NO_SUCH_BRANCH.to_string())),
        }
    }

    pub async fn create(
        &self,
        cmd: CreateBranch,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Branch> {
        // Step 1: cmd.from_branch exists
        if !self.repo.branch_exists(&cmd.from_branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        // Step 2: request_id is a Request id
        let trimmed_req = validate_request_id(&cmd.request_id)?.to_string();

        // Step 3: name is Text(60, required)
        let trimmed_name = validate_text(&cmd.name, 60, true)?.to_string();

        let from_branch = cmd.from_branch.clone();
        let from_branch_id = cmd.from_branch.as_str().to_string();
        let actor_str = actor.to_string();
        let stored_now = cut_to_millis(now);
        let repo = self.repo.clone();

        let main_id = BranchId::new("MAIN");
        let locked = main_id.clone();
        self.branch_tx
            .run(&locked, move |tx| {
                Box::pin(async move {
                    // The request may be given up in the branch it was sent
                    // from, under that branch's lock. Hold it too before
                    // anything is read: MAIN first (taken by `run`), then the
                    // sending branch — the one lock order of the system.
                    if from_branch != main_id {
                        BranchTx::lock_also(tx, &from_branch).await?;
                    }

                    // Step 4: a branch with this request_id exists:
                    // its name equals trimmed_name exactly -> return that branch, write nothing; otherwise REQUEST_CHANGED
                    if let Some(existing) = repo.find_by_request(tx, &trimmed_req).await? {
                        if existing.name == trimmed_name {
                            return Ok(existing);
                        } else {
                            return Err(AppError::Domain(REQUEST_CHANGED.to_string()));
                        }
                    }

                    // Step 5: the request is not Abandoned in from_branch
                    if repo.is_abandoned(tx, &from_branch_id, &trimmed_req).await? {
                        return Err(AppError::Domain(ABANDONED.to_string()));
                    }

                    // Step 6: no branch has this name, compared without regard to case (the main branch's XC-B counts)
                    if repo.name_taken(tx, &trimmed_name).await? {
                        return Err(AppError::Domain(NAME_TAKEN.to_string()));
                    }

                    // Step 7: fewer than 20 branches exist, the main one included
                    if repo.count(tx).await? >= 20 {
                        return Err(AppError::Domain(TOO_MANY_BRANCHES.to_string()));
                    }

                    // Insert
                    let new_id = format!("B{}", trimmed_req);
                    repo.insert(
                        tx,
                        &new_id,
                        &trimmed_name,
                        stored_now,
                        &actor_str,
                        &trimmed_req,
                    )
                    .await?;

                    Ok(Branch {
                        id: BranchId::new(&new_id),
                        name: trimmed_name,
                        request_id: Some(trimmed_req),
                        code: Some(new_id),
                        address: None,
                        timezone: "Asia/Bangkok".to_string(),
                        opening_hours: None,
                        booking_enabled: false,
                        contact_channels: Vec::new(),
                        status: BranchStatus::Active,
                    })
                })
            })
            .await
    }

    /// Save `branch`'s profile in that branch's `BranchTx`, so its version moves. An unknown
    /// zone, malformed hours or an empty code is `AppError::Validation`; a code another branch of
    /// the organisation holds is `AppError::Conflict`; an unknown branch is `NO_SUCH_BRANCH`.
    /// Nothing is written on a refusal.
    pub async fn save_profile(
        &self,
        branch: &BranchId,
        profile: BranchProfile,
    ) -> AppResult<Branch> {
        // Refuse what needs no lock before taking one, so a refusal writes no `branch_versions` row.
        validate_profile(&profile)?;
        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }
        let service = self.clone();
        let id = branch.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { service.save_profile_in(tx, &id, profile).await })
            })
            .await
    }

    /// `save_profile`'s work inside the caller's transaction, for a caller that already holds
    /// the branch's `BranchTx` (the v3 command seam runs `branchSave` in one, so a second
    /// `BranchTx` on the same branch would wait on its lock). Same refusals, nothing written on one.
    /// `save_profile` is this inside its own `BranchTx`.
    pub async fn save_profile_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        profile: BranchProfile,
    ) -> AppResult<Branch> {
        self.save_profile_with_contact_in(tx, branch, profile, None, None)
            .await
    }

    /// Saves `branch`'s profile inside `tx`, updating `contact_channels` and `status` when given
    /// and keeping the stored values when `None` (S3 #3406 B4).
    pub async fn save_profile_with_contact_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        profile: BranchProfile,
        contact_channels: Option<Vec<ContactChannel>>,
        status: Option<BranchStatus>,
    ) -> AppResult<Branch> {
        let code = validate_profile(&profile)?;
        let validated_channels = match contact_channels.as_deref() {
            Some(channels) => Some(ContactChannel::validate_list(channels)?),
            None => None,
        };
        let Some(current) = self.repo.find_for_update_in(tx, branch).await? else {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        };
        if self.repo.code_taken_in(tx, branch, &code).await? {
            return Err(AppError::Conflict(format!(
                "branch code {code:?} is taken in this organisation"
            )));
        }
        let next_channels = validated_channels.unwrap_or_else(|| current.contact_channels.clone());
        let next_status = status.unwrap_or(current.status);
        self.repo
            .update_profile_in(
                tx,
                branch,
                &code,
                profile.address.as_deref(),
                &profile.timezone,
                profile.opening_hours.as_ref(),
                profile.booking_enabled,
                &next_channels,
                next_status,
            )
            .await?;
        Ok(Branch {
            code: Some(code),
            address: profile.address,
            timezone: profile.timezone,
            opening_hours: profile.opening_hours,
            booking_enabled: profile.booking_enabled,
            contact_channels: next_channels,
            status: next_status,
            ..current
        })
    }
}

/// Checks a profile before anything is read or written, answering its trimmed code: a non-empty
/// code, an IANA zone and hours of `BranchHours`' shape (design §6), else `AppError::Validation`.
fn validate_profile(profile: &BranchProfile) -> AppResult<String> {
    let code = profile.code.trim();
    if code.is_empty() {
        return Err(AppError::Validation("branch code is required".to_string()));
    }
    BranchHours::new(&profile.timezone, profile.opening_hours.as_ref())?;
    Ok(code.to_string())
}
