//! `GET /api/v3/views/my_profile` (#654 CI-7, design `customer-identity-consent.md` §4.4, K8): a signed-in
//! customer's own profile and current consents.
//!
//! A named `?branch=` is checked first, for every caller, as the view core does: an unknown one is `noSuchBranch`
//! (#500). Then it answers through the route table (`customerSelf`): no session is `401 signIn`, staff or the owner
//! `403 roleNotAllowedHere`, a CUSTOMER user with no linked customer `403 outOfScope`. `?customer=` naming anyone
//! but the caller's own customer is `403 outOfScope` (ruling 7 of 2026-10-10 on #654). The shared view seam has no
//! way for `load` to answer that refusal, and a customer has no branch to version by, so this view registers its
//! own `GET` (as `my_queue_ticket` does) and answers the seam's envelope with `version: 0, changed: true` every
//! time (`VERSIONED = false`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::email_sender::EmailSender;
use xc_kernel::ids::BranchId;

use crate::application::copy_by_email::member_email::{MemberEmails, PgMemberEmails};
use crate::bootstrap::AppState;
use crate::infrastructure::notify::NotifyEmailSender;
use crate::presentation::dto::v3::common::ViewAnswer;
use crate::presentation::dto::v3::my_profile::{
    EmailCopy, MyConsent, MyProfileData, MyProfileFilter,
};
use crate::presentation::http::v3::command::{out_of_scope, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, authenticate_and_authorize, parse_branch, require_branch,
};

fn mask_email(address: &str) -> String {
    match address.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().unwrap_or('*');
            format!("{first}***@{domain}")
        }
        None => "***".to_owned(),
    }
}

pub struct MyProfileView {
    db: PgPool,
}

type ProfileRow = (String, String, String, String, String, String);
type ConsentRow = (String, Option<String>, bool, i32, String, DateTime<Utc>);

impl MyProfileView {
    /// The profile of `customer`; `NotFound` when there is none.
    async fn profile(&self, customer: &str) -> AppResult<MyProfileData> {
        let row: Option<ProfileRow> = sqlx::query_as(
            "SELECT id, organization_id::text, customer_no, display_name, preferred_language, status FROM customers WHERE id = $1",
        )
        .bind(customer)
        .fetch_optional(&self.db)
        .await
        .map_err(AppError::Database)?;
        let (customer_id, org_id, customer_no, display_name, preferred_language, status) = row
            .ok_or_else(|| AppError::NotFound {
                entity: "customer",
                id: customer.to_owned(),
            })?;
        let mut tx = self.db.begin().await.map_err(AppError::Database)?;
        let offered_languages = ConsentService::offered_languages_in(&mut tx, &org_id).await?;
        let consents: Vec<ConsentRow> = sqlx::query_as(
            "SELECT DISTINCT ON (purpose, channel) purpose, channel, granted, policy_version, lang, effective_at \
             FROM consents WHERE customer_id = $1 \
             ORDER BY purpose, channel, effective_at DESC, created_at DESC",
        )
        .bind(customer)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        let to = match xc_customer::get_pii_keys() {
            Ok(keys) => match PgMemberEmails::new(self.db.clone(), keys)
                .verified_email(customer)
                .await
            {
                Ok(v) => v.map(|v| mask_email(&v.address)),
                Err(AppError::Internal(reason)) => {
                    tracing::warn!(customer_id = %customer, %reason, "my_profile email decrypt failed");
                    None
                }
                Err(e) => return Err(e),
            },
            Err(_) => None,
        };
        let available = to.is_some() && NotifyEmailSender::from_env().is_configured();
        Ok(MyProfileData {
            customer_id,
            customer_no,
            display_name,
            preferred_language,
            status,
            offered_languages,
            consents: consents
                .into_iter()
                .map(
                    |(purpose, channel, granted, policy_version, lang, effective_at)| MyConsent {
                        purpose,
                        channel,
                        granted,
                        policy_version,
                        lang,
                        effective_at: effective_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    },
                )
                .collect(),
            email_copy: EmailCopy { available, to },
        })
    }
}

impl BranchView for MyProfileView {
    const NAME: &'static str = "my_profile";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "customerSelf");
    const VERSIONED: bool = false;
    type Data = MyProfileData;
    type Filter = MyProfileFilter;

    /// The caller's own profile; a filter naming another customer is `NotFound` here (the handler refuses it
    /// first, as `outOfScope`).
    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let own = who
            .customer_id
            .as_deref()
            .ok_or_else(|| AppError::NotFound {
                entity: "customer",
                id: String::new(),
            })?;
        if filter.customer.as_deref().is_some_and(|c| c != own) {
            return Err(AppError::NotFound {
                entity: "customer",
                id: String::new(),
            });
        }
        self.profile(own).await
    }
}

async fn handle(req: HttpRequest, view: web::Data<Arc<MyProfileView>>) -> HttpResponse {
    // A named branch first, for every caller, as the core checks it: an unknown one is `noSuchBranch` (#500). A
    // customer names none.
    let named =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .ok()
            .and_then(|q| q.into_inner().remove("branch"));
    match parse_branch(named.as_deref()) {
        Ok(Some(branch)) => {
            if let Err(e) = require_branch(&req, &branch).await {
                return e.error_response();
            }
        }
        Ok(None) => {}
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    }
    let who = match authenticate_and_authorize(&req, MyProfileView::INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let filter = match web::Query::<MyProfileFilter>::from_query(req.query_string()) {
        Ok(q) => q.into_inner(),
        Err(_) => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    // The route row passed, so the caller has a linked customer; any other one named is out of scope.
    let Some(own) = who.customer_id.as_deref() else {
        return out_of_scope().error_response();
    };
    if filter.customer.as_deref().is_some_and(|c| c != own) {
        return out_of_scope().error_response();
    }
    match view.profile(own).await {
        Ok(data) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer {
                version: 0,
                changed: true,
                data: Some(data),
            }),
        Err(e) => {
            tracing::error!(error = %e, "my_profile failed");
            server_error().error_response()
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(MyProfileView {
        db: state.db.clone(),
    });
    cfg.service(
        web::resource(format!("/api/v3/views/{}", MyProfileView::NAME))
            .app_data(web::Data::new(view))
            .route(web::get().to(handle)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyProfileData::export_all(&cfg)?;
    MyProfileFilter::export_all(&cfg)?;
    Ok(())
}
