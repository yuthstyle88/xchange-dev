//! Customer `my_bookings` view (BK-6, #1388).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{header, StatusCode};
use actix_web::{web, HttpRequest, HttpResponse, ResponseError};
use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::qr_token::QrToken;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::ViewAnswer;
use crate::presentation::dto::v3::my_bookings::{
    MyBookingItemDto, MyBookingsData, MyBookingsFilter,
};
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    authenticate_and_authorize, parse_branch, require_branch, BranchView,
};

pub struct MyBookingsView {
    pool: PgPool,
}

impl MyBookingsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type MyBookingDbRow = (
    String,                  // b.id
    String,                  // b.booking_no
    String,                  // b.status
    String,                  // b.customer_id
    String,                  // b.branch_id
    uuid::Uuid,              // b.slot_id
    i32,                     // b.qr_version
    DateTime<Utc>,           // s.start_at
    DateTime<Utc>,           // s.end_at
    Option<String>,          // qt.display_number
    i32,                     // p.cancel_cutoff_minutes
);

impl BranchView for MyBookingsView {
    const NAME: &'static str = "my_bookings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_bookings");
    const VERSIONED: bool = true;
    type Data = MyBookingsData;
    type Filter = MyBookingsFilter;

    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::Forbidden => Some(out_of_scope()),
            _ => None,
        }
    }

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let customer_id = who.customer_id.clone().unwrap_or_default();
        let requested_booking_id = filter.booking_id.clone();

        async move {
            // If specific booking requested, ensure it belongs to this customer
            if let Some(ref target_id) = requested_booking_id {
                let owner_opt: Option<(String,)> = sqlx::query_as(
                    "SELECT customer_id FROM bookings WHERE id = $1",
                )
                .bind(target_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                match owner_opt {
                    Some((owner,)) if owner == customer_id => {}
                    _ => return Err(AppError::Forbidden),
                }
            }

            let pii_keys = xc_customer::get_pii_keys().map_err(|e| AppError::Internal(e.to_string()))?;
            let qr_key = QrToken::key_from_pii(&pii_keys);

            let rows: Vec<MyBookingDbRow> = sqlx::query_as(
                "SELECT b.id, b.booking_no, b.status, b.customer_id, b.branch_id, \
                        b.slot_id, b.qr_version, s.start_at, s.end_at, \
                        (SELECT qt.display_number FROM queue_tickets qt \
                         WHERE qt.booking_id = b.id AND qt.status NOT IN ('CANCELLED', 'NO_SHOW') \
                         ORDER BY qt.created_at DESC LIMIT 1), \
                        COALESCE(p.cancel_cutoff_minutes, 60) \
                 FROM bookings b \
                 JOIN booking_slots s ON s.id = b.slot_id \
                 JOIN branches br ON br.id = b.branch_id \
                 LEFT JOIN booking_policies p ON p.organization_id = br.organization_id \
                 WHERE b.customer_id = $1 \
                   AND ($2 = '' OR b.branch_id = $2) \
                   AND ($3::text IS NULL OR b.id = $3) \
                 ORDER BY s.start_at DESC, b.booking_no DESC",
            )
            .bind(&customer_id)
            .bind(branch.as_str())
            .bind(&requested_booking_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let bookings = rows
                .into_iter()
                .map(|(id, booking_no, status, _cust_id, branch_id, slot_id, qr_version, start_at, end_at, ticket_no, cutoff_mins)| {
                    let booking_uuid = Uuid::parse_str(id.strip_prefix("BK-").unwrap_or(&id))
                        .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes()));
                    let qr_token = (status != "CANCELLED" && status != "COMPLETED" && status != "NO_SHOW" && status != "EXPIRED")
                        .then(|| QrToken::derive(&qr_key, &booking_uuid, qr_version as u32));

                    let cancel_cutoff_at = (start_at - Duration::minutes(cutoff_mins as i64)).to_rfc3339();

                    MyBookingItemDto {
                        id,
                        booking_no,
                        status,
                        branch_id,
                        slot_id: slot_id.to_string(),
                        start_at: start_at.to_rfc3339(),
                        end_at: end_at.to_rfc3339(),
                        qr_token,
                        quote_summary: "เรทสุดท้ายยืนยันเมื่อทำรายการ".to_string(),
                        cancel_cutoff_at: Some(cancel_cutoff_at),
                        ticket_number: ticket_no,
                        readiness: "unknown".to_string(),
                    }
                })
                .collect();

            Ok(MyBookingsData { bookings })
        }
    }
}

async fn handle(req: HttpRequest, view: web::Data<Arc<MyBookingsView>>) -> HttpResponse {
    let named = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    )
    .ok()
    .and_then(|q| q.into_inner().remove("branch"));
    let branch = match parse_branch(named.as_deref()) {
        Ok(Some(branch)) => {
            if let Err(e) = require_branch(&req, &branch).await {
                return e.error_response();
            }
            branch
        }
        Ok(None) => BranchId::new(""),
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    let who = match authenticate_and_authorize(&req, MyBookingsView::INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let Some(_own_customer_id) = who.customer_id.as_deref() else {
        return out_of_scope().error_response();
    };

    let filter = web::Query::<MyBookingsFilter>::from_query(req.query_string())
        .map(|q| q.into_inner())
        .unwrap_or_default();

    match view.load(&branch, &who, &filter).await {
        Ok(data) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer {
                version: 0,
                changed: true,
                data: Some(data),
            }),
        Err(AppError::Forbidden) => out_of_scope().error_response(),
        Err(e) => {
            tracing::error!(error = %e, "my_bookings query failed");
            V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response()
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(MyBookingsView::new(state.db.clone()));
    cfg.service(
        web::resource("/api/v3/views/my_bookings")
            .app_data(web::Data::new(view))
            .route(web::get().to(handle)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyBookingsData::export_all(&cfg)?;
    MyBookingsFilter::export_all(&cfg)?;
    Ok(())
}
