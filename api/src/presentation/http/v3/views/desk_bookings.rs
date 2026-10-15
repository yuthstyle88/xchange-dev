//! Staff `desk_bookings` view (BK-6, #1388).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

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

type DeskDbRow = (
    String,                  // b.id
    String,                  // b.booking_no
    String,                  // b.status
    String,                  // b.customer_id
    Option<String>,          // c.display_name
    Option<String>,          // ci.key_id
    Option<Vec<u8>>,         // ci.encrypted_value
    Option<String>,          // qt.display_number
    uuid::Uuid,              // b.slot_id
    DateTime<Utc>,           // s.start_at
    DateTime<Utc>,           // s.end_at
);

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

            let rows: Vec<DeskDbRow> = sqlx::query_as(
                "SELECT b.id, b.booking_no, b.status, b.customer_id, \
                        c.display_name, ci.key_id, ci.encrypted_value, \
                        (SELECT qt.display_number FROM queue_tickets qt \
                         WHERE qt.booking_id = b.id AND qt.status NOT IN ('CANCELLED', 'NO_SHOW') \
                         ORDER BY qt.created_at DESC LIMIT 1), \
                        b.slot_id, s.start_at, s.end_at \
                 FROM bookings b \
                 JOIN booking_slots s ON s.id = b.slot_id \
                 LEFT JOIN customers c ON c.id = b.customer_id \
                 LEFT JOIN customer_identities ci ON ci.customer_id = b.customer_id AND ci.type = 'PHONE' AND ci.status = 'ACTIVE' \
                 WHERE b.branch_id = $1 \
                   AND ($2::bytea IS NULL OR b.qr_token_hash = $2) \
                 ORDER BY s.start_at ASC, b.booking_no ASC",
            )
            .bind(branch.as_str())
            .bind(qr_hash)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let items = rows
                .into_iter()
                .map(|(id, booking_no, status, customer_id, display_name, key_id, enc_phone, ticket_no, slot_id, start_at, end_at)| {
                    let phone_last4 = match (key_id, enc_phone) {
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
                        id,
                        booking_no,
                        status,
                        customer_display_name: display_name,
                        phone_last4,
                        readiness: "unknown".to_string(),
                        ticket_number: ticket_no,
                        slot_id: slot_id.to_string(),
                        start_at: start_at.to_rfc3339(),
                        end_at: end_at.to_rfc3339(),
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
