//! Branch application service.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::branch::commands::create_branch::CreateBranch;
use crate::domain::branch::entities::branch::Branch;
use crate::domain::validation::{
    ABANDONED, NAME_TAKEN, NO_SUCH_BRANCH, REQUEST_CHANGED, TOO_MANY_BRANCHES, cut_to_millis,
    validate_request_id, validate_text,
};

type BranchRow = (String, String, Option<String>);

#[derive(Clone)]
pub struct BranchService {
    branch_tx: BranchTx,
}

impl BranchService {
    pub fn new(branch_tx: BranchTx) -> Self {
        Self { branch_tx }
    }

    pub async fn list(&self) -> AppResult<Vec<Branch>> {
        let rows = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id FROM branches ORDER BY (id = 'MAIN') DESC, created_at ASC, id ASC",
        )
        .fetch_all(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|(id, name, request_id)| Branch {
                id: BranchId::new(id),
                name,
                request_id,
            })
            .collect())
    }

    pub async fn require(&self, id: &BranchId) -> AppResult<Branch> {
        let row = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id FROM branches WHERE id = $1",
        )
        .bind(id.as_str())
        .fetch_optional(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((id, name, request_id)) => Ok(Branch {
                id: BranchId::new(id),
                name,
                request_id,
            }),
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
        let from_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                .bind(cmd.from_branch.as_str())
                .fetch_one(self.branch_tx.pool())
                .await
                .map_err(AppError::Database)?;

        if !from_exists {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        // Step 2: request_id is a Request id
        let trimmed_req = validate_request_id(&cmd.request_id)?.to_string();

        // Step 3: name is Text(60, required)
        let trimmed_name = validate_text(&cmd.name, 60, true)?.to_string();

        let from_branch_id = cmd.from_branch.as_str().to_string();
        let actor_str = actor.to_string();
        let stored_now = cut_to_millis(now);

        let main_id = BranchId::new("MAIN");
        self.branch_tx
            .run(&main_id, move |tx| {
                Box::pin(async move {
                    // Step 4: a branch with this request_id exists:
                    // its name equals trimmed_name exactly -> return that branch, write nothing; otherwise REQUEST_CHANGED
                    let existing = sqlx::query_as::<_, BranchRow>(
                        "SELECT id, name, request_id FROM branches WHERE request_id = $1",
                    )
                    .bind(&trimmed_req)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    if let Some((id, name, request_id)) = existing {
                        if name == trimmed_name {
                            return Ok(Branch {
                                id: BranchId::new(id),
                                name,
                                request_id,
                            });
                        } else {
                            return Err(AppError::Domain(REQUEST_CHANGED.to_string()));
                        }
                    }

                    // Step 5: the request is not Abandoned in from_branch
                    let abandoned: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
                    )
                    .bind(&from_branch_id)
                    .bind(&trimmed_req)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    if abandoned {
                        return Err(AppError::Domain(ABANDONED.to_string()));
                    }

                    // Step 6: no branch has this name, compared without regard to case (the main branch's XC-B counts)
                    let name_taken: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM branches WHERE lower(name) = lower($1))",
                    )
                    .bind(&trimmed_name)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    if name_taken {
                        return Err(AppError::Domain(NAME_TAKEN.to_string()));
                    }

                    // Step 7: fewer than 20 branches exist, the main one included
                    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM branches")
                        .fetch_one(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;

                    if count >= 20 {
                        return Err(AppError::Domain(TOO_MANY_BRANCHES.to_string()));
                    }

                    // Insert
                    let new_id = format!("B{}", trimmed_req);
                    sqlx::query(
                        "INSERT INTO branches (id, name, created_at, created_by, request_id) VALUES ($1, $2, $3, $4, $5)",
                    )
                    .bind(&new_id)
                    .bind(&trimmed_name)
                    .bind(stored_now)
                    .bind(&actor_str)
                    .bind(&trimmed_req)
                    .execute(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

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
