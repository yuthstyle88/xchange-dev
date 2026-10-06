//! Parsing ops* requests into CustodyCommand.

use pos_error::{AppError, AppResult};
use serde_json::Value;
use sha2::{Digest, Sha256};
use xc_kernel::money::{CurrencyCode, Money, Side};

use crate::domain::custody::commands::custody_command::{ClockDirection, CustodyCommand};
use crate::domain::custody::commands::proposal::Proposal;
use crate::domain::custody::value_objects::count_sheet::CountSheet;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::denomination_row::DenominationRow;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use crate::domain::shift::value_objects::balances::Balances;

pub const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
pub const BAD_KIND: &str = "ประเภทงานไม่ถูกต้อง";
pub const EVIDENCE_COUNT: &str = "แนบหลักฐาน 1\u{2013}5 ไฟล์";

fn get_str<'a>(val: &'a Value, key: &str) -> &'a str {
    val.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

pub fn parse_amount(val: Option<&Value>) -> AppResult<Money> {
    match val {
        None | Some(Value::Null) => Err(AppError::Domain(CustodyError::AmountMissing.to_string())),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Err(AppError::Domain(CustodyError::AmountMissing.to_string()))
            } else {
                Money::parse(trimmed)
                    .map_err(|_| AppError::Domain(CustodyError::AmountInvalid.to_string()))
            }
        }
        Some(Value::Number(n)) => {
            let s = n.to_string();
            Money::parse(&s).map_err(|_| AppError::Domain(CustodyError::AmountInvalid.to_string()))
        }
        Some(_) => Err(AppError::Domain(CustodyError::AmountInvalid.to_string())),
    }
}

pub fn parse_balances(val: &Value) -> AppResult<Balances> {
    let map = match val {
        Value::Object(m) => Some(m),
        _ => None,
    };
    let mut pairs = Vec::new();
    for &code in CurrencyCode::all() {
        let entry = map.and_then(|m| m.get(code.as_str()));
        match entry {
            None | Some(Value::Null) => {}
            Some(v) => {
                let m = parse_amount(Some(v))?;
                if !m.is_zero() {
                    pairs.push((code, m));
                }
            }
        }
    }
    Ok(Balances::of(&pairs))
}

fn canonical_json_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut btree = std::collections::BTreeMap::new();
            for (k, val) in map {
                btree.insert(k.clone(), canonical_json_value(val));
            }
            let mut new_map = serde_json::Map::new();
            for (k, val) in btree {
                new_map.insert(k, val);
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_json_value).collect()),
        other => other.clone(),
    }
}

pub fn compute_payload_hash(payload: &Value) -> String {
    let canonical = canonical_json_value(payload);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn parse_count_sheet(
    totals_val: Option<&Value>,
    denoms_val: Option<&Value>,
) -> AppResult<CountSheet> {
    let totals = match totals_val {
        Some(v) => parse_balances(v)?,
        None => Balances::empty(),
    };

    let denominations = match denoms_val {
        Some(Value::Array(arr)) => {
            let mut rows = Vec::with_capacity(arr.len());
            for item in arr {
                let curr_str = item
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let currency = CurrencyCode::parse(curr_str)
                    .map_err(|_| AppError::Domain(CustodyError::DenominationInvalid.to_string()))?;

                let denom_val = item.get("denomination");
                let denomination = match denom_val {
                    Some(Value::Number(n)) => Money::parse(&n.to_string()).map_err(|_| {
                        AppError::Domain(CustodyError::DenominationInvalid.to_string())
                    })?,
                    _ => {
                        return Err(AppError::Domain(
                            CustodyError::DenominationInvalid.to_string(),
                        ));
                    }
                };

                let count_val = item.get("count");
                let count = match count_val {
                    Some(Value::Number(n)) if n.is_u64() => {
                        let u = n.as_u64().unwrap();
                        if u <= u32::MAX as u64 {
                            u as u32
                        } else {
                            return Err(AppError::Domain(
                                CustodyError::DenominationInvalid.to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(AppError::Domain(
                            CustodyError::DenominationInvalid.to_string(),
                        ));
                    }
                };

                rows.push(DenominationRow {
                    currency,
                    denomination,
                    count,
                });
            }
            rows
        }
        _ => Vec::new(),
    };

    Ok(CountSheet {
        totals,
        denominations,
    })
}

pub fn parse_command(action: &str, payload: &Value) -> AppResult<CustodyCommand> {
    match action {
        "opsSetEntity" => {
            let company = get_str(payload, "company").to_string();
            let site = get_str(payload, "site").to_string();
            let confirm_separated =
                matches!(payload.get("confirmSeparated"), Some(Value::Bool(true)));
            Ok(CustodyCommand::SetEntity {
                company,
                site,
                confirm_separated,
            })
        }
        "opsEnable" => Ok(CustodyCommand::Enable),
        "opsCutoff" => Ok(CustodyCommand::Cutoff),
        "opsClock" => {
            let direction = match payload.get("direction").and_then(|v| v.as_str()) {
                Some("IN") => ClockDirection::In,
                Some("OUT") => ClockDirection::Out,
                _ => return Err(AppError::Domain(BAD_REQUEST.to_string())),
            };
            Ok(CustodyCommand::Clock(direction))
        }
        "opsBeginTransaction" => {
            let side_str = payload.get("type").and_then(|v| v.as_str());
            let side = match side_str {
                Some("BUY") => Side::Buy,
                Some("SELL") => Side::Sell,
                _ => return Err(AppError::Domain(CustodyError::DraftInvalid.to_string())),
            };
            let curr_str = payload.get("currency").and_then(|v| v.as_str());
            let currency = match curr_str.and_then(|s| CurrencyCode::parse(s).ok()) {
                Some(c) => c,
                None => return Err(AppError::Domain(CustodyError::DraftInvalid.to_string())),
            };
            let amount = parse_amount(payload.get("amount"))?;
            Ok(CustodyCommand::BeginTransaction {
                side,
                currency,
                amount,
            })
        }
        "opsTarget" => {
            let amounts = match payload.get("amounts") {
                Some(v) => parse_balances(v)?,
                None => parse_balances(&Value::Null)?,
            };
            Ok(CustodyCommand::SetTarget { amounts })
        }
        "opsStartService" => {
            let equipment_ready = matches!(payload.get("equipmentReady"), Some(Value::Bool(true)));
            let rate_updated_opt = payload.get("rateUpdated").and_then(|v| v.as_str());
            let (rates_verified, rate_updated) = match rate_updated_opt {
                Some(s) => (
                    matches!(payload.get("ratesVerified"), Some(Value::Bool(true))),
                    s.to_string(),
                ),
                None => (false, String::new()),
            };
            Ok(CustodyCommand::StartService {
                equipment_ready,
                rates_verified,
                rate_updated,
            })
        }
        "opsPropose" => {
            let kind_str = payload
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let kind = match kind_str {
                "OPEN" => RequestKind::Open,
                "HANDOVER" => RequestKind::Handover,
                "CLOSE" => RequestKind::Close,
                "SAFE_OPEN" => RequestKind::SafeOpen,
                "SAFE_IN" => RequestKind::SafeIn,
                "SAFE_OUT" => RequestKind::SafeOut,
                "EXTERNAL_IN" => RequestKind::ExternalIn,
                "EXTERNAL_OUT" => RequestKind::ExternalOut,
                _ => return Err(AppError::Domain(BAD_KIND.to_string())),
            };

            let evidence_keys_arr = match payload.get("evidenceKeys") {
                Some(Value::Array(arr)) if arr.len() <= 5 => arr,
                _ => return Err(AppError::Domain(EVIDENCE_COUNT.to_string())),
            };
            let evidence_keys: Vec<String> = evidence_keys_arr
                .iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect();

            let request_id = get_str(payload, "requestId").to_string();
            let note = get_str(payload, "note").to_string();
            let receiver = payload
                .get("receiverId")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);

            let payload_hash = compute_payload_hash(payload);

            let (counted, safe_count) = match kind {
                RequestKind::Open | RequestKind::SafeOpen => {
                    let counted =
                        parse_count_sheet(payload.get("counted"), payload.get("denominations"))?;
                    (counted, CountSheet::empty())
                }
                RequestKind::Handover | RequestKind::Close => {
                    let counted =
                        parse_count_sheet(payload.get("counted"), payload.get("denominations"))?;
                    let safe_count = parse_count_sheet(
                        payload.get("safeCount"),
                        payload.get("safeDenominations"),
                    )?;
                    (counted, safe_count)
                }
                _ => (CountSheet::empty(), CountSheet::empty()),
            };

            let (currency, amount) = if kind.is_transfer() {
                let curr = payload
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .and_then(|s| CurrencyCode::parse(s).ok());
                let amt_val = payload.get("amount");
                let amt = match amt_val {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) if s.trim().is_empty() => None,
                    Some(v) => Some(parse_amount(Some(v))?),
                };
                (curr, amt)
            } else {
                (None, None)
            };

            Ok(CustodyCommand::Propose(Box::new(Proposal {
                request_id,
                payload_hash,
                kind,
                receiver,
                note,
                evidence_keys,
                counted,
                safe_count,
                currency,
                amount,
            })))
        }
        "opsHold" => {
            let customer_id = get_str(payload, "customerId").to_string();
            let reason = get_str(payload, "reason").to_string();
            Ok(CustodyCommand::Hold {
                customer_id,
                reason,
            })
        }
        "opsRelease" => {
            let hold_id = get_str(payload, "id").to_string();
            let note = get_str(payload, "note").to_string();
            Ok(CustodyCommand::Release { hold_id, note })
        }
        "opsReject" => {
            let id = get_str(payload, "id").to_string();
            let reason = get_str(payload, "reason").to_string();
            Ok(CustodyCommand::Reject { id, reason })
        }
        "opsSign" => {
            let id = get_str(payload, "id").to_string();
            Ok(CustodyCommand::Sign { id })
        }
        "opsDispatch" => {
            let id = get_str(payload, "id").to_string();
            Ok(CustodyCommand::Dispatch { id })
        }
        "opsCommit" => {
            let id = get_str(payload, "id").to_string();
            Ok(CustodyCommand::Commit { id })
        }
        "opsReconcile" => {
            let id = get_str(payload, "id").to_string();
            let note = get_str(payload, "note").to_string();
            Ok(CustodyCommand::Reconcile { id, note })
        }
        _ => Err(AppError::Domain(BAD_REQUEST.to_string())),
    }
}
