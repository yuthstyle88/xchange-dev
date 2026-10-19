//! Customer `my_bookings` view (BK-6, #1388).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::effective_status::{OverdueFacts, effective_status};
use xc_booking::domain::booking::qr_token::QrToken;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::status::status_service::{StatusService, TicketRef};
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::ViewAnswer;
use crate::presentation::dto::v3::my_bookings::{
    MyBookingItemDto, MyBookingsData, MyBookingsFilter,
};
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, authenticate_and_authorize, parse_branch, require_branch,
};

pub struct MyBookingsView {
    pool: PgPool,
}

impl MyBookingsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct MyBookingDbRow {
    id: String,
    booking_no: String,
    status: String,
    branch_id: String,
    slot_id: Uuid,
    qr_version: i32,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    ticket_id: Option<Uuid>,
    ticket_no: Option<String>,
    ticket_service_type: Option<String>,
    ticket_sequence: Option<i32>,
    ticket_status: Option<String>,
    cutoff_mins: i32,
    confirm_expires_at: Option<DateTime<Utc>>,
    grace_until: DateTime<Utc>,
    service_date: NaiveDate,
    branch_today: NaiveDate,
    has_active_ticket: bool,
}

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
        let customer_id = who.customer_id.clone();
        let requested_booking_id = filter.booking_id.clone();

        async move {
            let customer_id = customer_id.ok_or_else(|| AppError::NotFound {
                entity: "customer",
                id: String::new(),
            })?;

            // If specific booking requested, ensure it belongs to this customer
            if let Some(ref target_id) = requested_booking_id {
                let owner_opt: Option<(String,)> =
                    sqlx::query_as("SELECT customer_id FROM bookings WHERE id = $1")
                        .bind(target_id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(AppError::Database)?;

                match owner_opt {
                    Some((owner,)) if owner == customer_id => {}
                    _ => return Err(AppError::Forbidden),
                }
            }

            let pii_keys =
                xc_customer::get_pii_keys().map_err(|e| AppError::Internal(e.to_string()))?;
            let qr_key = QrToken::key_from_pii(&pii_keys);
            let now = Utc::now();

            let rows: Vec<MyBookingDbRow> = sqlx::query_as(
                "SELECT b.id, b.booking_no, b.status, b.branch_id, \
                        b.slot_id, b.qr_version, s.start_at, s.end_at, \
                        qt.ticket_id, qt.ticket_no, qt.ticket_service_type, \
                        qt.ticket_sequence, qt.ticket_status, \
                        COALESCE(p.cancel_cutoff_minutes, 60) AS cutoff_mins, \
                        b.confirm_expires_at, \
                        (s.start_at + (COALESCE(p.no_show_grace_minutes, 15) * interval '1 minute')) AS grace_until, \
                        s.service_date, \
                        ($4 AT TIME ZONE COALESCE(NULLIF(br.timezone, ''), 'Asia/Bangkok'))::date AS branch_today, \
                        EXISTS ( \
                            SELECT 1 FROM queue_tickets qt_active \
                            WHERE qt_active.booking_id = b.id \
                              AND qt_active.status IN ('WAITING', 'CALLED', 'SERVING') \
                        ) AS has_active_ticket \
                 FROM bookings b \
                 JOIN booking_slots s ON s.id = b.slot_id \
                 JOIN branches br ON br.id = b.branch_id \
                 LEFT JOIN booking_policies p ON p.organization_id = br.organization_id \
                 LEFT JOIN LATERAL ( \
                     SELECT q.id AS ticket_id, \
                            q.display_number AS ticket_no, \
                            q.service_type AS ticket_service_type, \
                            q.sequence AS ticket_sequence, \
                            q.status AS ticket_status \
                     FROM queue_tickets q \
                     WHERE q.booking_id = b.id AND q.status NOT IN ('CANCELLED', 'NO_SHOW') \
                     ORDER BY q.created_at DESC \
                     LIMIT 1 \
                 ) qt ON true \
                 WHERE b.customer_id = $1 \
                   AND ($2 = '' OR b.branch_id = $2) \
                   AND ($3::text IS NULL OR b.id = $3) \
                 ORDER BY s.start_at DESC, b.booking_no DESC",
            )
            .bind(&customer_id)
            .bind(branch.as_str())
            .bind(&requested_booking_id)
            .bind(now)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
            let status_service =
                StatusService::new(BranchTx::new(Transactor::new(pool.clone())), repo);
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            let mut bookings = Vec::with_capacity(rows.len());

            for row in rows {
                let facts = OverdueFacts {
                    confirm_expires_at: row.confirm_expires_at,
                    grace_until: row.grace_until,
                    service_date: row.service_date,
                    branch_today: row.branch_today,
                    has_active_ticket: row.has_active_ticket,
                };
                let status = BookingStatus::parse(&row.status)
                    .map(|st| effective_status(st, &facts, now).as_str().to_string())
                    .unwrap_or(row.status);

                let booking_uuid = Uuid::parse_str(row.id.strip_prefix("BK-").unwrap_or(&row.id))
                    .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, row.id.as_bytes()));
                let qr_token = (status != "CANCELLED"
                    && status != "COMPLETED"
                    && status != "NO_SHOW"
                    && status != "EXPIRED")
                    .then(|| QrToken::derive(&qr_key, &booking_uuid, row.qr_version as u32));

                let cancel_cutoff_at =
                    (row.start_at - Duration::minutes(row.cutoff_mins as i64)).to_rfc3339();

                let (tickets_ahead, estimate, estimate_as_of) = match (
                    row.ticket_id,
                    row.ticket_service_type.as_deref(),
                    row.ticket_sequence,
                    row.ticket_status.as_deref(),
                ) {
                    (Some(ticket_id), Some(service_type), Some(sequence), Some(ticket_status)) => {
                        let row_branch = BranchId::new(&row.branch_id);
                        let (ahead, est) = status_service
                            .ticket_wait_in(
                                &mut tx,
                                TicketRef {
                                    id: ticket_id,
                                    branch: &row_branch,
                                    service_type,
                                    sequence,
                                    status: ticket_status,
                                },
                                now,
                            )
                            .await?;
                        let as_of = (ticket_status == "WAITING").then(|| now.to_rfc3339());
                        (Some(ahead), est, as_of)
                    }
                    _ => (None, None, None),
                };

                bookings.push(MyBookingItemDto {
                    id: row.id,
                    booking_no: row.booking_no,
                    status,
                    branch_id: row.branch_id,
                    slot_id: row.slot_id.to_string(),
                    start_at: row.start_at.to_rfc3339(),
                    end_at: row.end_at.to_rfc3339(),
                    qr_token,
                    quote_summary: "เรทสุดท้ายยืนยันเมื่อทำรายการ".to_string(),
                    cancel_cutoff_at: Some(cancel_cutoff_at),
                    ticket_number: row.ticket_no,
                    tickets_ahead,
                    estimate_seconds: estimate.map(|e| e.seconds),
                    estimate_low_seconds: estimate.map(|e| e.low_seconds),
                    estimate_high_seconds: estimate.map(|e| e.high_seconds),
                    estimate_as_of,
                    readiness: "unknown".to_string(),
                });
            }
            tx.rollback().await.map_err(AppError::Database)?;

            Ok(MyBookingsData { bookings })
        }
    }
}

async fn handle(req: HttpRequest, view: web::Data<Arc<MyBookingsView>>) -> HttpResponse {
    let named =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
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
