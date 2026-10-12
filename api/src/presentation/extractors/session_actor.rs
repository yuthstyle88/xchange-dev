//! The one place a v3 session becomes an `access::Actor` (#654 CI-7; design
//! `docs/design/blueprint/customer-identity-consent.md` K7). The lane controller's ruling 5 of 2026-10-10 on #654
//! puts it here, not in `SessionService`: `xc-customer` depends on `xc-auth`, so the session service cannot reach
//! the customer link without a crate cycle.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use xc_auth::application::session::SessionService;
use xc_auth::domain::access::{Actor, ScopedRole};
use xc_auth::domain::user::User;
use xc_customer::application::customer_link::customer_of_user;

/// The actor of the live session `session_token`, with `customer_id` filled from `customers.user_id`
/// (`None` for every user with no linked customer: staff, the owner, MARKETING, a CUSTOMER role with no customer),
/// working in the session's effective branch (SB-1 §14.3). `None` when the token names no live session.
pub async fn resolve_session_actor(
    session: &SessionService,
    pool: &PgPool,
    session_token: &str,
    now: DateTime<Utc>,
) -> Option<Actor> {
    let active = match session.active_session(session_token, now).await {
        Ok(active) => active?,
        Err(e) => {
            tracing::error!(code = e.refusal().1, "reading the session failed");
            return None;
        }
    };
    let branch = active.effective_branch(now);
    actor_of_user(pool, &active.user, active.roles)
        .await
        .map(|actor| actor.with_branch(branch))
}

/// The actor of a signed-in `user` with its valid `roles`, `customer_id` filled from `customers.user_id`. `None`
/// when the link cannot be read: a caller whose customer is unknown is not signed in, rather than a customer
/// with none.
pub async fn actor_of_user(pool: &PgPool, user: &User, roles: Vec<ScopedRole>) -> Option<Actor> {
    match customer_of_user(pool, &user.id).await {
        Ok(customer) => Some(Actor::new(
            &user.id,
            &user.organization_id,
            roles,
            customer.as_deref(),
        )),
        Err(e) => {
            tracing::error!(error = %e, "reading the user's customer failed");
            None
        }
    }
}
