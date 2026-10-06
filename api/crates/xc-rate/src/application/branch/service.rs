//! Branch application service.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::branch::commands::create_branch::CreateBranch;
use crate::domain::branch::entities::branch::Branch;
use crate::domain::branch::repositories::branch_repository::BranchRepository;
use crate::domain::validation::{
    ABANDONED, NAME_TAKEN, NO_SUCH_BRANCH, REQUEST_CHANGED, TOO_MANY_BRANCHES, cut_to_millis,
    validate_request_id, validate_text,
};
use crate::infrastructure::branch::branch_repository::PgBranchRepository;

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
                        id: BranchId::new(new_id),
                        name: trimmed_name,
                        request_id: Some(trimmed_req),
                    })
                })
            })
            .await
    }
}
