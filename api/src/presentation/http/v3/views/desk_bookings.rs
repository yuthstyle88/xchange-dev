//! Staff `desk_bookings` view (BK-6, #1388).

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::effective_status::{OverdueFacts, effective_status};
use xc_kernel::ids::BranchId;
use xc_verification::application::readiness::Readiness;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::desk_bookings::{
    DeskBookingItemDto, DeskBookingsData, DeskBookingsFilter,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct DeskBookingsView {
    pool: PgPool,
}

impl DeskBookingsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct DeskDbRow {
    id: String,
    booking_no: String,
    status: String,
    customer_id: String,
    display_name: Option<String>,
    key_id: Option<String>,
    encrypted_value: Option<Vec<u8>>,
    ticket_no: Option<String>,
    slot_id: uuid::Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    confirm_expires_at: Option<DateTime<Utc>>,
    grace_until: DateTime<Utc>,
    service_date: NaiveDate,
    branch_today: NaiveDate,
    has_active_ticket: bool,
}

impl BranchView for DeskBookingsView {
    const NAME: &'static str = "desk_bookings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "desk_bookings");
    const VERSIONED: bool = true;
    type Data = DeskBookingsData;
    type Filter = DeskBookingsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let qr_filter = filter.qr.clone();

        async move {
            let qr_hash = qr_filter.as_ref().map(|q| Sha256::digest(q.as_bytes()).to_vec());

            let pii_keys = xc_customer::get_pii_keys().ok();
            let cipher = pii_keys.map(xc_kernel::pii::PiiCipher::new);
            let now = Utc::now();

            let rows: Vec<DeskDbRow> = sqlx::query_as(
                "SELECT b.id, b.booking_no, b.status, b.customer_id, \
                        c.display_name, ci.key_id, ci.encrypted_value, \
                        (SELECT qt.display_number FROM queue_tickets qt \
                         WHERE qt.booking_id = b.id AND qt.status NOT IN ('CANCELLED', 'NO_SHOW') \
                         ORDER BY qt.created_at DESC LIMIT 1) AS ticket_no, \
                        b.slot_id, s.start_at, s.end_at, \
                        b.confirm_expires_at, \
                        (s.start_at + (COALESCE(p.no_show_grace_minutes, 15) * interval '1 minute')) AS grace_until, \
                        s.service_date, \
                        ($3 AT TIME ZONE COALESCE(NULLIF(br.timezone, ''), 'Asia/Bangkok'))::date AS branch_today, \
                        EXISTS ( \
                            SELECT 1 FROM queue_tickets qt \
                            WHERE qt.booking_id = b.id \
                              AND qt.status IN ('WAITING', 'CALLED', 'SERVING') \
                        ) AS has_active_ticket \
                 FROM bookings b \
                 JOIN booking_slots s ON s.id = b.slot_id \
                 JOIN branches br ON br.id = b.branch_id \
                 LEFT JOIN booking_policies p ON p.organization_id = br.organization_id \
                 LEFT JOIN customers c ON c.id = b.customer_id \
                 LEFT JOIN customer_identities ci ON ci.customer_id = b.customer_id AND ci.type = 'PHONE' AND ci.status = 'ACTIVE' \
                 WHERE b.branch_id = $1 \
                   AND ($2::bytea IS NULL OR b.qr_token_hash = $2) \
                 ORDER BY s.start_at ASC, b.booking_no ASC",
            )
            .bind(branch.as_str())
            .bind(qr_hash)
            .bind(now)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut readiness_by_customer: HashMap<String, &'static str> = HashMap::new();
            for row in &rows {
                if !readiness_by_customer.contains_key(&row.customer_id) {
                    let r = Readiness::of(&pool, &row.customer_id)
                        .await
                        .map_err(|e| {
                            tracing::error!(error = %e, "readiness query error");
                            pos_error::AppError::Internal("failed to query readiness".into())
                        })?
                        .as_str();
                    readiness_by_customer.insert(row.customer_id.clone(), r);
                }
            }

            let items = rows
                .into_iter()
                .map(|row| {
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

                    let customer_id = row.customer_id;
                    let readiness = readiness_by_customer
                        .get(&customer_id)
                        .copied()
                        .unwrap_or("NONE")
                        .to_string();
                    let phone_last4 = match (row.key_id, row.encrypted_value) {
                        (Some(k), Some(bytes)) => (|| {
                            let c = cipher.as_ref()?;
                            let sealed = xc_kernel::pii::Sealed { key_id: k, bytes };
                            let aad = format!("{customer_id}PHONE");
                            let decrypted = c.decrypt(aad.as_bytes(), &sealed).ok()?;
                            let phone_str = String::from_utf8(decrypted).ok()?;
                            let char_count = phone_str.chars().count();
                            if char_count >= 4 {
                                Some(phone_str.chars().skip(char_count - 4).collect())
                            } else {
                                None
                            }
                        })(),
                        _ => None,
                    };

                    DeskBookingItemDto {
                        id: row.id,
                        booking_no: row.booking_no,
                        status,
                        customer_display_name: row.display_name,
                        phone_last4,
                        readiness,
                        ticket_number: row.ticket_no,
                        slot_id: row.slot_id.to_string(),
                        start_at: row.start_at.to_rfc3339(),
                        end_at: row.end_at.to_rfc3339(),
                    }
                })
                .collect();

            Ok(DeskBookingsData { items })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(DeskBookingsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeskBookingsData::export_all(&cfg)?;
    DeskBookingsFilter::export_all(&cfg)?;
    Ok(())
}
