//! Kiosk DTOs: JSON representations matching v2 byte-for-byte and key order.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde::Deserialize;
use serde_json::Value;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CompatRate, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use xc_kiosk::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
use xc_kiosk::domain::kiosk::value_objects::notifications::Notifications;
use xc_kiosk::domain::kiosk::value_objects::public_rates::PublicRates;
use xc_kiosk::domain::kiosk::value_objects::queue_page::QueuePage;
use xc_kiosk::domain::kiosk::value_objects::request_identity::RequestIdentity;
use xc_kiosk::domain::kiosk::value_objects::work_times::WorkTimes;
use xc_rate::domain::rate::entities::counter_rate::CounterRate;

use crate::application::compat::v2_json::{
    V2Json, money_json, rate_json, time_json, time_or_empty_json,
};
use crate::application::settings::payment_json::rules_v2_json;
use crate::presentation::dto::v3::common::RateFreshnessDto;

#[derive(Debug, Clone, Deserialize)]
pub struct CurrencyTableEntry {
    pub code: String,
    pub name: String,
    #[serde(rename = "nameEn")]
    pub name_en: String,
    pub countries: Value,
}

static CURRENCIES: LazyLock<Vec<CurrencyTableEntry>> = LazyLock::new(|| {
    let raw: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/kiosk/currency_table.json"
    ))
    .expect("currency_table.json must be valid JSON");
    let currencies = raw["currencies"].clone();
    serde_json::from_value(currencies).expect("currencies array")
});

pub fn currency_table() -> &'static [CurrencyTableEntry] {
    &CURRENCIES
}

static CURRENCY_MAP: LazyLock<HashMap<String, CurrencyTableEntry>> = LazyLock::new(|| {
    CURRENCIES
        .iter()
        .cloned()
        .map(|entry| (entry.code.clone(), entry))
        .collect()
});

fn optional<T>(value: Option<T>, print: impl FnOnce(T) -> V2Json) -> V2Json {
    value.map_or(V2Json::Null, print)
}

/// A kiosk time as v2 printed a PostgreSQL `timestamptz`: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub(crate) fn kiosk_time_json(at: KioskTime) -> V2Json {
    DateTime::<Utc>::from_timestamp_millis(at.epoch_millis()).map_or(V2Json::Null, time_json)
}

pub(crate) fn kiosk_time_iso(at: KioskTime) -> Option<String> {
    match kiosk_time_json(at) {
        V2Json::Text(s) => Some(s),
        _ => None,
    }
}

/// `startedAt`, `completedAt`, `durationSeconds` of a request.
fn work_times_json(times: &WorkTimes) -> [(&'static str, V2Json); 3] {
    [
        ("startedAt", optional(times.started_at, kiosk_time_json)),
        ("completedAt", optional(times.completed_at, kiosk_time_json)),
        (
            "durationSeconds",
            optional(times.duration_seconds(), V2Json::int),
        ),
    ]
}

/// v2's queue label: `K-` and the first eight characters of the id, upper-cased.
pub(crate) fn queue_of(id: &str) -> String {
    let first_eight: String = id.chars().take(8).collect();
    format!("K-{}", first_eight.to_uppercase())
}

#[derive(Debug, Clone)]
pub struct EmployeeInfo {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: String,
}

fn shortest(decimal: &str) -> String {
    if !decimal.contains('.') {
        return decimal.to_string();
    }
    decimal
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn compat_rate_json(rate: Rate, unit_size: UnitSize) -> AppResult<V2Json> {
    let dec = CompatRate::per_one(rate, unit_size)
        .map_err(|e| AppError::Internal(format!("kiosk compat rate error: {e}")))?;
    Ok(V2Json::Number(shortest(&dec.to_string())))
}

pub fn rates_json(public_rates: &PublicRates, freshness: &RateFreshnessDto) -> AppResult<V2Json> {
    let mut rate_items = Vec::with_capacity(public_rates.rates.len());
    for rate in &public_rates.rates {
        let entry = CURRENCY_MAP.get(rate.code.as_str());
        let name = entry.map_or("", |e| &e.name);
        let name_en = entry.map_or("", |e| &e.name_en);
        let countries = entry.map_or_else(
            || V2Json::Array(vec![]),
            |e| V2Json::from_value(&e.countries),
        );
        rate_items.push(V2Json::object(vec![
            ("code", V2Json::text(rate.code.as_str())),
            ("name", V2Json::text(name)),
            ("nameEn", V2Json::text(name_en)),
            ("countries", countries),
            ("buy", compat_rate_json(rate.buy, rate.unit_size)?),
            ("sell", compat_rate_json(rate.sell, rate.unit_size)?),
        ]));
    }

    let freshness_json = V2Json::object(vec![
        ("stale", V2Json::Bool(freshness.stale)),
        (
            "sourceDate",
            optional(freshness.source_date.as_ref(), |d| V2Json::text(d.as_str())),
        ),
        (
            "publishedAt",
            optional(freshness.published_at.as_ref(), |p| {
                V2Json::text(p.as_str())
            }),
        ),
    ]);

    Ok(V2Json::object(vec![
        ("branchId", V2Json::text(public_rates.branch_id.as_str())),
        ("branchName", V2Json::text(&public_rates.branch_name)),
        ("open", V2Json::Bool(public_rates.open)),
        ("rates", V2Json::Array(rate_items)),
        (
            "paymentMethods",
            rules_v2_json(&public_rates.payment, false),
        ),
        ("simulation", V2Json::Bool(false)),
        ("freshness", freshness_json),
    ]))
}

pub fn quote_json(
    req: &KioskRequest,
    identity: &RequestIdentity,
    branch_name: &str,
) -> AppResult<V2Json> {
    let rate_v2 = compat_rate_json(req.quote.rate, req.quote.unit_size)?;
    Ok(V2Json::object(vec![
        ("id", V2Json::text(&identity.id)),
        ("queue", V2Json::text(&identity.queue)),
        ("orderNo", V2Json::text(&identity.order_no)),
        (
            "transactionNo",
            optional(identity.transaction_no.as_ref(), |t| {
                V2Json::text(t.as_str())
            }),
        ),
        ("status", V2Json::text(identity.status.as_str())),
        (
            "paymentMethod",
            V2Json::text(req.quote.payment_method.as_str()),
        ),
        (
            "type",
            V2Json::text(match req.quote.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            }),
        ),
        ("currency", V2Json::text(req.quote.currency.as_str())),
        ("rate", rate_v2),
        ("foreign", money_json(req.quote.foreign)),
        ("thb", money_json(req.quote.thb)),
        ("inputAmount", money_json(req.quote.input_amount)),
        ("branchName", V2Json::text(branch_name)),
    ]))
}

pub fn identity_json(identity: &RequestIdentity) -> V2Json {
    V2Json::object(vec![
        ("id", V2Json::text(&identity.id)),
        ("queue", V2Json::text(&identity.queue)),
        ("orderNo", V2Json::text(&identity.order_no)),
        (
            "transactionNo",
            optional(identity.transaction_no.as_ref(), |t| {
                V2Json::text(t.as_str())
            }),
        ),
        ("status", V2Json::text(identity.status.as_str())),
    ])
}

pub fn customer_receipt_json(receipt: &CustomerReceipt) -> V2Json {
    V2Json::object(vec![
        (
            "image",
            optional(receipt.image.as_ref(), |img| V2Json::text(img.as_str())),
        ),
        ("id", V2Json::text(&receipt.identity.id)),
        ("queue", V2Json::text(&receipt.identity.queue)),
        ("orderNo", V2Json::text(&receipt.identity.order_no)),
        (
            "transactionNo",
            optional(receipt.identity.transaction_no.as_ref(), |t| {
                V2Json::text(t.as_str())
            }),
        ),
        ("status", V2Json::text(receipt.identity.status.as_str())),
    ])
}

/// `notifications`, in v2's key order. `username_of` turns a holder's id into the name printed.
pub fn notifications_json(
    branch_id: &BranchId,
    notifications: &Notifications,
    employee: &EmployeeInfo,
    server_now: DateTime<Utc>,
    username_of: &dyn Fn(&str) -> String,
) -> V2Json {
    let working: Vec<V2Json> = notifications
        .working
        .iter()
        .map(|item| {
            V2Json::object(vec![
                ("id", V2Json::text(&item.id)),
                ("queue", V2Json::text(queue_of(&item.id))),
                ("staff", V2Json::text(username_of(&item.staff))),
                ("startedAt", optional(item.started_at, kiosk_time_json)),
                ("status", V2Json::text(item.status.as_str())),
            ])
        })
        .collect();
    V2Json::object(vec![
        ("serverNow", time_json(server_now)),
        ("workingCount", V2Json::int(notifications.working_count)),
        ("working", V2Json::Array(working)),
        ("branchId", V2Json::text(branch_id.as_str())),
        ("branchName", V2Json::text(&notifications.branch_name)),
        ("count", V2Json::int(notifications.count)),
        (
            "latest",
            optional(notifications.latest.as_ref(), |l| V2Json::text(l.as_str())),
        ),
        (
            "employee",
            V2Json::object(vec![
                ("id", V2Json::text(&employee.id)),
                ("username", V2Json::text(&employee.username)),
                ("name", V2Json::text(&employee.name)),
                ("role", V2Json::text(&employee.role)),
            ]),
        ),
    ])
}

/// Who asks for the list: the id a claim is stored under, and whether the role may claim.
pub struct ListCaller<'a> {
    pub id: &'a str,
    pub claims: bool,
}

/// `list`, in v2's key order.
pub fn list_json(
    branch_id: &BranchId,
    page: &QueuePage,
    branch_name: &str,
    server_now: DateTime<Utc>,
    caller: &ListCaller<'_>,
    username_of: &dyn Fn(&str) -> String,
) -> AppResult<V2Json> {
    let mut items = Vec::with_capacity(page.items.len());
    for item in &page.items {
        let request = &item.request;
        let status = item.identity.status;
        let held_by_caller = request.staff.as_deref() == Some(caller.id);
        let can_accept = caller.claims && status == KioskStatus::Waiting;
        let can_continue = caller.claims
            && held_by_caller
            && matches!(status, KioskStatus::Accepted | KioskStatus::IdChecked);
        let rate_v2 = compat_rate_json(request.quote.rate, request.quote.unit_size)?;
        let mut pairs = vec![
            (
                "paymentMethod",
                V2Json::text(request.quote.payment_method.as_str()),
            ),
            (
                "type",
                V2Json::text(match request.quote.side {
                    Side::Buy => "BUY",
                    Side::Sell => "SELL",
                }),
            ),
            ("currency", V2Json::text(request.quote.currency.as_str())),
            ("rate", rate_v2),
            ("foreign", money_json(request.quote.foreign)),
            ("thb", money_json(request.quote.thb)),
            ("inputAmount", money_json(request.quote.input_amount)),
            ("branchName", V2Json::text(branch_name)),
            ("id", V2Json::text(&item.identity.id)),
            ("branch", V2Json::text(branch_id.as_str())),
            ("status", V2Json::text(status.as_str())),
            ("created", V2Json::int(request.created.epoch_millis())),
            (
                "staff",
                V2Json::text(
                    request
                        .staff
                        .as_deref()
                        .map_or_else(String::new, username_of),
                ),
            ),
            ("queue", V2Json::text(&item.identity.queue)),
            ("orderNo", V2Json::text(&item.identity.order_no)),
            (
                "transactionNo",
                optional(item.identity.transaction_no.as_ref(), |t| {
                    V2Json::text(t.as_str())
                }),
            ),
        ];
        pairs.extend(work_times_json(&item.times));
        pairs.push(("canAccept", V2Json::Bool(can_accept)));
        pairs.push(("canContinue", V2Json::Bool(can_continue)));
        items.push(V2Json::object(pairs));
    }

    Ok(V2Json::object(vec![
        ("branchId", V2Json::text(branch_id.as_str())),
        ("serverNow", time_json(server_now)),
        ("page", V2Json::int(i64::from(page.page))),
        ("hasMore", V2Json::Bool(page.has_more)),
        ("requests", V2Json::Array(items)),
    ]))
}

/// `accept` when the caller has just claimed the request.
pub fn accept_claimed_json(times: &WorkTimes) -> V2Json {
    let mut pairs = vec![
        ("saved", V2Json::Bool(true)),
        ("claimed", V2Json::Bool(true)),
    ];
    pairs.extend(work_times_json(times));
    V2Json::object(pairs)
}

/// `accept` when the caller already holds the request.
pub fn accept_mine_json(times: &WorkTimes) -> V2Json {
    let mut pairs = vec![("saved", V2Json::Bool(true))];
    pairs.extend(work_times_json(times));
    V2Json::object(pairs)
}

pub fn get_json(req: &KioskRequest, branch_name: &str) -> AppResult<V2Json> {
    let queue = queue_of(&req.id);
    let rate_v2 = compat_rate_json(req.quote.rate, req.quote.unit_size)?;
    Ok(V2Json::object(vec![
        (
            "paymentMethod",
            V2Json::text(req.quote.payment_method.as_str()),
        ),
        (
            "type",
            V2Json::text(match req.quote.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            }),
        ),
        ("currency", V2Json::text(req.quote.currency.as_str())),
        ("rate", rate_v2),
        ("foreign", money_json(req.quote.foreign)),
        ("thb", money_json(req.quote.thb)),
        ("inputAmount", money_json(req.quote.input_amount)),
        ("branchName", V2Json::text(branch_name)),
        ("id", V2Json::text(&req.id)),
        ("queue", V2Json::text(&queue)),
    ]))
}

pub fn manage_json(switches: &[CurrencySwitch], rates: &[CounterRate]) -> V2Json {
    let switch_map: HashMap<String, bool> = switches
        .iter()
        .map(|s| (s.code.as_str().to_string(), s.enabled))
        .collect();
    let rate_map: HashMap<String, &CounterRate> = rates
        .iter()
        .map(|r| (r.code.as_str().to_string(), r))
        .collect();

    let items: Vec<V2Json> = CURRENCIES
        .iter()
        .map(|entry| {
            let rate_opt = rate_map.get(&entry.code).copied();
            let enabled = switch_map.get(&entry.code).copied().unwrap_or(false);

            let mut pairs = vec![
                ("code", V2Json::text(&entry.code)),
                ("name", V2Json::text(&entry.name)),
                ("nameEn", V2Json::text(&entry.name_en)),
                ("countries", V2Json::from_value(&entry.countries)),
            ];

            if let Some(symbol) = rate_opt
                .and_then(|r| r.symbol.as_deref())
                .filter(|s| !s.is_empty())
            {
                pairs.push(("symbol", V2Json::text(symbol)));
            }

            pairs.extend([
                ("buy", optional(rate_opt.and_then(|r| r.buy), rate_json)),
                ("sell", optional(rate_opt.and_then(|r| r.sell), rate_json)),
                (
                    "updated",
                    rate_opt.map_or_else(|| V2Json::text(""), |r| time_or_empty_json(r.updated_at)),
                ),
                (
                    "requestId",
                    V2Json::text(
                        rate_opt
                            .and_then(|r| r.request_id.as_deref())
                            .unwrap_or_default(),
                    ),
                ),
                (
                    "partnerName",
                    V2Json::text(rate_opt.map_or("", |r| r.partner_name.as_str())),
                ),
                (
                    "partnerCost",
                    optional(rate_opt.and_then(|r| r.partner_cost), rate_json),
                ),
                (
                    "botReference",
                    optional(
                        rate_opt.and_then(|r| r.reference.as_ref()),
                        V2Json::from_value,
                    ),
                ),
                ("enabled", V2Json::Bool(enabled)),
            ]);

            V2Json::object(pairs)
        })
        .collect();

    V2Json::object(vec![("currencies", V2Json::Array(items))])
}

#[cfg(test)]
mod tests {
    use xc_kernel::money::{CurrencyCode, MinorUnit, Rate, UnitSize};
    use xc_kiosk::domain::kiosk::value_objects::counter_facts::LineId;
    use xc_kiosk::domain::kiosk::value_objects::counter_view::CounterRate;

    use super::*;

    #[test]
    fn non_power_of_10_unit_gives_internal_not_a_number() {
        let err = compat_rate_json(Rate::parse("23.1").unwrap(), UnitSize::parse("3").unwrap())
            .unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));

        let public_rates = PublicRates {
            branch_id: BranchId::new("MAIN"),
            branch_name: "Main Branch".to_string(),
            open: true,
            payment: Default::default(),
            rates: vec![CounterRate {
                code: CurrencyCode::parse("USD").unwrap(),
                buy: Rate::parse("35.0").unwrap(),
                sell: Rate::parse("36.0").unwrap(),
                unit_size: UnitSize::parse("3").unwrap(),
                minor_unit: MinorUnit::new(2).unwrap(),
                line_id: LineId(1),
            }],
        };
        let err = rates_json(&public_rates, &RateFreshnessDto::default()).unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }
}
