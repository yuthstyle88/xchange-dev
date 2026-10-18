//! The v3 endpoint seam's caller (APP-18 #1373; design `docs/design/blueprint/customer-app.md` §9 APP-18 row, §9.2
//! #1943; the lane controller's rulings on #1373 and #1943: `Caller { user, app_session, session_hash }`, no legacy
//! actor; option B, the `*_as` methods sit alongside today's `&access::Actor` methods).

use actix_web::{HttpRequest, HttpResponse, ResponseError};
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSession, AppSessionId};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;

use super::error::V3Error;
use crate::presentation::pages;

/// Who called a v3 route: the signed-in user, if any, the anonymous app session the request carried, if any
/// (`xc_app_sid`; the seam reads an existing one and never issues one, ruling 4), and the hash of the caller's live
/// `__Host-xc_session` row, if any (#1943, §9.2).
#[derive(Debug, Clone)]
pub struct Caller {
    pub user: Option<Actor>,
    pub app_session: Option<AppSession>,
    pub session_hash: Option<String>,
}

/// Resolves the caller for a v3 route, checking policy admission.
///
/// Ruling 1: Admission without a session is allowed ONLY on `EntryPath::Rpc` when `Need::Public`.
/// Non-Public rows and Kiosk Public rows without session still answer `signIn` (401).
///
/// Ruling 2: A CUSTOMER-only user reaches the handler with `caller.user = Some(actor)` where `actor.customer_id` is set.
///
/// Ruling 4: Reads `APP_SESSION_COOKIE` (`xc_app_sid`) if present and valid; never issues one and never sends `Set-Cookie`.
pub(crate) async fn authenticate_caller(
    req: &HttpRequest,
    inherits: (EntryPath, &'static str),
) -> Result<Caller, HttpResponse> {
    let callers = pages::get_callers(req).await;
    let now = chrono::Utc::now();
    let action = Action::new(inherits.1);

    let user = match inherits.0 {
        EntryPath::StaffRpc => callers.staff_user,
        _ => callers.user,
    };

    let session_hash = crate::presentation::extractors::actor::caller_session_hash(req, now).await;

    let user = match user {
        Some(u) => {
            if let Some(ref hash) = session_hash {
                let pool = req
                    .app_data::<actix_web::web::Data<crate::bootstrap::AppState>>()
                    .map(|s| &s.db)
                    .or_else(|| {
                        req.app_data::<actix_web::web::Data<xc_auth::application::session::SessionService>>()
                            .map(|s| s.pool())
                    });
                if let Some(pool) = pool {
                    match sqlx::query_scalar::<_, bool>(
                        "SELECT must_change_password FROM user_sessions WHERE hash = $1 AND expires > $2",
                    )
                    .bind(hash)
                    .bind(now)
                    .fetch_optional(pool)
                    .await
                    {
                        Ok(Some(true)) => {
                            return Err(V3Error::new(
                                "passwordChangeRequired",
                                None,
                                "password change required",
                                actix_web::http::StatusCode::FORBIDDEN,
                            )
                            .error_response());
                        }
                        Ok(_) => {}
                        Err(e) => {
                            // Fail closed, but never echo the database's error text to the client.
                            tracing::error!(error = %e, "forced-change gate: reading the session flag failed");
                            return Err(super::command::server_error().error_response());
                        }
                    }
                }
            }
            if let Err(e) = Policy::authorize(Some(&u), inherits.0, &action, None, now) {
                return Err(V3Error::from(e).error_response());
            }
            Some(u)
        }
        None => {
            if inherits.0 == EntryPath::Rpc
                && Policy::need(inherits.0, &action) == Some(Need::Public)
            {
                None
            } else {
                let err = Policy::authorize(None, inherits.0, &action, None, now)
                    .err()
                    .unwrap_or(PolicyError::SignIn);
                return Err(V3Error::from(err).error_response());
            }
        }
    };

    let app_session = req
        .cookie(APP_SESSION_COOKIE)
        .and_then(|c| AppSessionId::parse(c.value()))
        .map(|id| {
            let hash = id.hash();
            AppSession { id, hash }
        });

    Ok(Caller {
        user,
        app_session,
        session_hash,
    })
}
