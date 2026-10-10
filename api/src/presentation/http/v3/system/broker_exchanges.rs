//! `GET /api/v3/system/broker/exchanges?since=<cursor>&limit=<n>` (FD-3 #974; design
//! `docs/design/blueprint/broker-seam.md` §6.4, the wire of Broker-Platform `broker-platform.md`
//! §12.3): the page Broker's `XchangeSource` pulls, behind `X-System-Token`.
//!
//! Scaffold written by the ticket author: `register` adds no route yet. The implementer fills it
//! and calls it from `configure_v3`.

use std::sync::Arc;

use actix_web::http::header::CACHE_CONTROL;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::SecondsFormat;
use serde::Serialize;
use xc_referral::domain::feed_cursor::FeedCursor;
use xc_referral::domain::feed_reader::{ExchangeFeedReader, MAX_LIMIT};
use xc_referral::infrastructure::feed_reader::PgExchangeFeedReader;

use super::token::{HEADER, SystemToken};
use crate::bootstrap::AppState;

/// The route's path.
pub const PATH: &str = "/api/v3/system/broker/exchanges";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerFeedItem {
    id: String,
    customer_sub: Option<String>,
    phone: Option<String>,
    status: &'static str,
    value_satang: i64,
    reversed_satang: i64,
    completed_at: String,
    reversed_at: Option<String>,
    updated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerFeedPage {
    items: Vec<BrokerFeedItem>,
    next_cursor: Option<String>,
}

#[derive(Clone)]
struct RouteState {
    token: Arc<SystemToken>,
    reader: Arc<dyn ExchangeFeedReader>,
}

/// Registers the route. Reads the token once (`SystemToken::from_env`).
pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let route_state = RouteState {
        token: Arc::new(SystemToken::from_env()),
        reader: Arc::new(PgExchangeFeedReader::new(state.db.clone())),
    };
    cfg.app_data(web::Data::new(route_state))
        .service(web::resource(PATH).route(web::get().to(get_broker_exchanges)));
}

async fn get_broker_exchanges(req: HttpRequest, state: web::Data<RouteState>) -> HttpResponse {
    let presented = req.headers().get(HEADER).map(|h| h.as_bytes());
    if !state.token.accepts(presented) {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "unauthorized"
        }));
    }

    let mut limit = 200u32;
    let mut since = None;

    let query_str = req.query_string();
    if !query_str.is_empty() {
        let pairs = match web::Query::<Vec<(String, String)>>::from_query(query_str) {
            Ok(p) => p.into_inner(),
            Err(_) => {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "invalid query string"
                }));
            }
        };

        for (k, v) in pairs {
            match k.as_str() {
                "limit" => {
                    let parsed: u32 = match v.parse() {
                        Ok(n) => n,
                        Err(_) => {
                            return HttpResponse::BadRequest().json(serde_json::json!({
                                "error": "invalid limit"
                            }));
                        }
                    };
                    if !(1..=MAX_LIMIT).contains(&parsed) {
                        return HttpResponse::BadRequest().json(serde_json::json!({
                            "error": format!("limit must be between 1 and {MAX_LIMIT}")
                        }));
                    }
                    limit = parsed;
                }
                "since" => {
                    let cursor = match FeedCursor::parse(&v) {
                        Ok(c) => c,
                        Err(_) => {
                            return HttpResponse::BadRequest().json(serde_json::json!({
                                "error": "invalid since cursor"
                            }));
                        }
                    };
                    since = Some(cursor);
                }
                _ => {}
            }
        }
    }

    let page = match state.reader.page(since, limit).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "broker exchanges feed read failed");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "internal error"
            }));
        }
    };

    let items = page
        .items
        .into_iter()
        .map(|item| BrokerFeedItem {
            id: item.id.to_string(),
            customer_sub: item.customer_sub,
            phone: item.phone,
            status: item.status.as_str(),
            value_satang: item.value_satang,
            reversed_satang: item.reversed_satang,
            completed_at: item
                .completed_at
                .to_rfc3339_opts(SecondsFormat::Micros, true),
            reversed_at: item
                .reversed_at
                .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Micros, true)),
            updated_at: item.updated_at.to_rfc3339_opts(SecondsFormat::Micros, true),
        })
        .collect();

    let next_cursor = page.next_cursor.map(|c| c.encode());

    HttpResponse::Ok()
        .insert_header((CACHE_CONTROL, "no-store"))
        .json(BrokerFeedPage { items, next_cursor })
}
