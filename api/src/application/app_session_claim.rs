//! Claiming an anonymous app session at sign-in (#998 APP-7, design `customer-app.md` P6; placement ruling
//! issuecomment-6137476393, signature ruling issuecomment-6138295640).
//!
//! When a request that carries an `xc_app_sid` cookie first has a customer, every planner draft and assistant
//! conversation of that session becomes the customer's, in one transaction. The claim lives here and not in
//! `xc-auth`'s session service because `xc-app` depends on `xc-auth`. A second claim of the same session moves
//! nothing, and that is not an error.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use xc_app::application::assistant::assistant_service::AssistantService;
use xc_app::application::planner::planner_service::PlannerService;
use xc_app::domain::app_error::CustomerAppError;
use xc_app::domain::app_session::SessionHash;
use xc_customer::application::customer_link::customer_of_user;

fn app_error(e: CustomerAppError) -> AppError {
    AppError::Internal(format!("app session claim: {}", e.code()))
}

/// Moves the drafts, then the conversations, of `session` to `customer_id` in `tx`. Answers
/// (drafts moved, conversations moved).
pub async fn claim_app_session_in(
    pool: &PgPool,
    tx: &mut Transaction<'_, Postgres>,
    session: &SessionHash,
    customer_id: &str,
    now: DateTime<Utc>,
) -> AppResult<(u64, u64)> {
    let drafts = PlannerService::new(pool.clone())
        .claim_session_in(tx, session, customer_id, now)
        .await
        .map_err(app_error)?;
    let conversations = AssistantService::new(pool.clone())
        .claim_session_in(tx, session, customer_id, now)
        .await
        .map_err(app_error)?;
    Ok((drafts, conversations))
}

/// The claim at a sign-in or a register: `session` goes to `customer_id` in a transaction of its own. The caller has
/// already signed the customer in, so a failure here is logged and does not undo the sign-in; the drafts stay with
/// the session and a later sign-in on the same cookie claims them.
pub async fn claim_session_for(
    pool: &PgPool,
    session: &SessionHash,
    customer_id: &str,
    now: DateTime<Utc>,
) {
    let result = async {
        let mut tx = pool.begin().await.map_err(AppError::Database)?;
        let moved = claim_app_session_in(pool, &mut tx, session, customer_id, now).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok::<_, AppError>(moved)
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "app session claim failed");
    }
}

/// The claim at a sign-in, where only the user is known: the user's customer, if any, takes `session`.
pub async fn claim_session_for_user(
    pool: &PgPool,
    session: &SessionHash,
    user_id: &str,
    now: DateTime<Utc>,
) {
    match customer_of_user(pool, user_id).await {
        Ok(Some(customer_id)) => claim_session_for(pool, session, &customer_id, now).await,
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, "app session claim: customer lookup failed"),
    }
}
