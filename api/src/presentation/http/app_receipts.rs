//! Ticket #1003 (APP-11, folded in #1012): customer receipt route `GET /api/app/receipts/{ref}`.
//! Design `docs/design/blueprint/customer-app.md` §5.1, §8, §9.1.

use std::path::PathBuf;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use sqlx::{PgPool, Row};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::authenticate_and_authorize;

pub const INHERITS: (EntryPath, &str) = (EntryPath::Rpc, "my_receipt");

pub struct AppReceiptsEndpoint {
    pub pool: PgPool,
    pub files_dir: PathBuf,
}

fn not_found_refusal() -> HttpResponse {
    V3Error::new("notFound", Some("ref"), "ไม่พบเอกสาร", StatusCode::NOT_FOUND).error_response()
}

fn server_error_refusal() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

async fn handle(req: HttpRequest, endpoint: web::Data<Arc<AppReceiptsEndpoint>>) -> HttpResponse {
    let who = match authenticate_and_authorize(&req, INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let Some(own_customer_id) = who.customer_id.as_deref() else {
        return out_of_scope().error_response();
    };

    let receipt_ref = req.match_info().get("ref").unwrap_or_default();
    if receipt_ref.is_empty() {
        return not_found_refusal();
    }

    let row = match sqlx::query(
        r#"
        SELECT r.document_ref, f.mime
        FROM receipts r
        JOIN transactions t ON t.id = r.transaction_id
        LEFT JOIN files f ON f.key = r.document_ref
        WHERE r.number = $1 AND t.customer_id = $2
        "#,
    )
    .bind(receipt_ref)
    .bind(own_customer_id)
    .fetch_optional(&endpoint.pool)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::error!(error = %e, "my_receipt query failed");
            return server_error_refusal();
        }
    };

    let Some(row) = row else {
        return not_found_refusal();
    };

    let doc_ref: String = row.get("document_ref");
    let mime: String = row
        .try_get::<String, _>("mime")
        .unwrap_or_else(|_| "text/html; charset=utf-8".to_string());

    let file_path = endpoint.files_dir.join(&doc_ref);
    let bytes = match tokio::fs::read(&file_path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, path = %file_path.display(), "failed to read receipt file");
            return not_found_refusal();
        }
    };

    HttpResponse::Ok().content_type(mime).body(bytes)
}

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let endpoint = Arc::new(AppReceiptsEndpoint {
        pool: state.db.clone(),
        files_dir: state.settings.files.dir.clone().into(),
    });
    cfg.service(
        web::resource("/api/app/receipts/{ref}")
            .app_data(web::Data::new(endpoint))
            .route(web::get().to(handle)),
    );
}
