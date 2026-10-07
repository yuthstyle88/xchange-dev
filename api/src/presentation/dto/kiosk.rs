//! Kiosk DTOs: JSON representations matching v2 byte-for-byte and key order.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use xc_kiosk::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
use xc_kiosk::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
use xc_kiosk::domain::kiosk::value_objects::notifications::Notifications;
use xc_kiosk::domain::kiosk::value_objects::public_rates::PublicRates;
use xc_kiosk::domain::kiosk::value_objects::request_identity::RequestIdentity;
use xc_rate::domain::rate::entities::counter_rate::CounterRate;

use crate::application::compat::v2_json::{V2Json, money_json, rate_json, time_or_empty_json};

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

#[derive(Debug, Clone)]
pub struct EmployeeInfo {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: String,
}

pub fn rates_json(public_rates: &PublicRates) -> V2Json {
    let rate_items: Vec<V2Json> = public_rates
        .rates
        .iter()
        .map(|rate| {
            let entry = CURRENCY_MAP.get(rate.code.as_str());
            let name = entry.map_or("", |e| &e.name);
            let name_en = entry.map_or("", |e| &e.name_en);
            let countries = entry.map_or_else(
                || V2Json::Array(vec![]),
                |e| V2Json::from_value(&e.countries),
            );
            V2Json::object(vec![
                ("code", V2Json::text(rate.code.as_str())),
                ("name", V2Json::text(name)),
                ("nameEn", V2Json::text(name_en)),
                ("countries", countries),
                ("buy", rate_json(rate.buy)),
                ("sell", rate_json(rate.sell)),
            ])
        })
        .collect();

    V2Json::object(vec![
        ("branchId", V2Json::text(public_rates.branch_id.as_str())),
        ("branchName", V2Json::text(&public_rates.branch_name)),
        ("open", V2Json::Bool(public_rates.open)),
        ("rates", V2Json::Array(rate_items)),
        ("simulation", V2Json::Bool(false)),
    ])
}

pub fn quote_json(req: &KioskRequest, identity: &RequestIdentity, branch_name: &str) -> V2Json {
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
        ("rate", rate_json(req.quote.rate)),
        ("foreign", money_json(req.quote.foreign)),
        ("thb", money_json(req.quote.thb)),
        ("inputAmount", money_json(req.quote.input_amount)),
        ("branchName", V2Json::text(branch_name)),
    ])
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

pub fn notifications_json(
    branch_id: &BranchId,
    notifications: &Notifications,
    employee: &EmployeeInfo,
) -> V2Json {
    V2Json::object(vec![
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

pub fn list_json(
    branch_id: &BranchId,
    requests: &[IdentifiedRequest],
    staff_names: &[String],
    branch_name: &str,
) -> V2Json {
    let items: Vec<V2Json> = requests
        .iter()
        .zip(staff_names)
        .map(|(item, staff)| {
            V2Json::object(vec![
                ("id", V2Json::text(&item.identity.id)),
                ("branch_id", V2Json::text(branch_id.as_str())),
                ("status", V2Json::text(item.identity.status.as_str())),
                ("created", V2Json::int(item.request.created.epoch_millis())),
                ("staff", V2Json::text(staff.as_str())),
                ("branch", V2Json::text(branch_id.as_str())),
                (
                    "paymentMethod",
                    V2Json::text(item.request.quote.payment_method.as_str()),
                ),
                (
                    "type",
                    V2Json::text(match item.request.quote.side {
                        Side::Buy => "BUY",
                        Side::Sell => "SELL",
                    }),
                ),
                (
                    "currency",
                    V2Json::text(item.request.quote.currency.as_str()),
                ),
                ("rate", rate_json(item.request.quote.rate)),
                ("foreign", money_json(item.request.quote.foreign)),
                ("thb", money_json(item.request.quote.thb)),
                ("inputAmount", money_json(item.request.quote.input_amount)),
                ("branchName", V2Json::text(branch_name)),
                ("queue", V2Json::text(&item.identity.queue)),
                ("orderNo", V2Json::text(&item.identity.order_no)),
                (
                    "transactionNo",
                    optional(item.identity.transaction_no.as_ref(), |t| {
                        V2Json::text(t.as_str())
                    }),
                ),
            ])
        })
        .collect();

    V2Json::object(vec![("requests", V2Json::Array(items))])
}

pub fn get_json(req: &KioskRequest, branch_name: &str) -> V2Json {
    let first_eight: String = req.id.chars().take(8).collect();
    let queue = format!("K-{}", first_eight.to_uppercase());
    V2Json::object(vec![
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
        ("rate", rate_json(req.quote.rate)),
        ("foreign", money_json(req.quote.foreign)),
        ("thb", money_json(req.quote.thb)),
        ("inputAmount", money_json(req.quote.input_amount)),
        ("branchName", V2Json::text(branch_name)),
        ("id", V2Json::text(&req.id)),
        ("queue", V2Json::text(&queue)),
    ])
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
