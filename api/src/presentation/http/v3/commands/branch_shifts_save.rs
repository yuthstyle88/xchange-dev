//! Ticket CS-9 (#6188): `branchShiftsSave` command (configurable-shifts.md §2.9).
//!
//! Saves the shift templates for a branch, replacing the live set.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{NaiveTime, Utc};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplateInput;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::branch_shifts::{BranchShiftsData, BranchShiftsSaveInput};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, server_error,
};
use crate::presentation::http::v3::error::V3Error;

pub struct BranchShiftsSaveCommand(pub PgPool);

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

fn shift_plan_invalid(reason: &str, detail: &str) -> V3Error {
    let message = match reason {
        "none" => "ต้องมีอย่างน้อยหนึ่งกะ".to_string(),
        "tooMany" => "กำหนดกะได้ไม่เกิน 12 กะ".to_string(),
        "emptyName" => "กรุณาตั้งชื่อกะ".to_string(),
        "nameTooLong" => "ชื่อกะยาวเกิน 40 ตัวอักษร".to_string(),
        "duplicateName" => "ชื่อกะซ้ำกัน".to_string(),
        "zeroLength" => "เวลาเริ่มและเวลาเลิกกะต้องไม่เท่ากัน".to_string(),
        "spanOver24h" => "กะทั้งหมดต้องอยู่ภายใน 24 ชั่วโมงนับจากกะแรก".to_string(),
        "gap" => {
            if detail.is_empty() {
                "มีช่วงเวลาที่ไม่มีกะ".to_string()
            } else {
                format!("ช่วง {detail} ไม่มีกะ")
            }
        }
        "breakIncomplete" => "กรุณาระบุเวลาพักให้ครบทั้งเริ่มและสิ้นสุด".to_string(),
        "breakOutsideShift" => "เวลาพักต้องอยู่ภายในเวลากะ".to_string(),
        "badTime" => "รูปแบบเวลาไม่ถูกต้อง".to_string(),
        _ => "ข้อมูลกะไม่ถูกต้อง".to_string(),
    };
    V3Error::new(
        "shiftPlanInvalid",
        None,
        message,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .with_details(serde_json::json!({
        "reason": reason,
        "detail": detail,
    }))
}

impl CommandAction for BranchShiftsSaveCommand {
    const NAME: &'static str = "branchShiftsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchSave");
    type Input = BranchShiftsSaveInput;
    type Output = BranchShiftsData;
    const ROUTE: &'static str = "/api/branches/shifts";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let mut inputs = Vec::with_capacity(input.templates.len());
        for (i, row) in input.templates.into_iter().enumerate() {
            let starts_at = parse_time(&row.starts_at)
                .ok_or_else(|| shift_plan_invalid("badTime", &i.to_string()))?;
            let ends_at = parse_time(&row.ends_at)
                .ok_or_else(|| shift_plan_invalid("badTime", &i.to_string()))?;

            let bs_opt = row
                .break_starts_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let be_opt = row
                .break_ends_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let break_window = match (bs_opt, be_opt) {
                (Some(bs_str), Some(be_str)) => {
                    let bs = parse_time(bs_str)
                        .ok_or_else(|| shift_plan_invalid("badTime", &i.to_string()))?;
                    let be = parse_time(be_str)
                        .ok_or_else(|| shift_plan_invalid("badTime", &i.to_string()))?;
                    Some((bs, be))
                }
                (None, None) => None,
                _ => return Err(shift_plan_invalid("breakIncomplete", &i.to_string())),
            };

            inputs.push(ShiftTemplateInput {
                id: row.id.filter(|s| !s.trim().is_empty()),
                name: row.name,
                starts_at,
                ends_at,
                break_window,
            });
        }

        let org: String = match sqlx::query_scalar(
            "SELECT organization_id::text FROM branches WHERE id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?
        {
            Some(o) => o,
            None => who.organization_id.clone(),
        };

        let branch_tx = BranchTx::new(Transactor::new(self.0.clone()));
        let service = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx);

        let plan = service
            .save_in(tx, branch, &org, inputs, Utc::now())
            .await
            .map_err(|e| match e {
                pos_error::AppError::Validation(msg) if msg.starts_with("shiftPlanInvalid:") => {
                    let rest = &msg["shiftPlanInvalid:".len()..];
                    let (reason, detail) = rest.split_once(':').unwrap_or((rest, ""));
                    shift_plan_invalid(reason, detail)
                }
                pos_error::AppError::Validation(msg) if msg == "shiftTemplateUnknown" => V3Error::new(
                    "shiftTemplateUnknown",
                    None,
                    "ไม่พบกะที่ต้องการแก้ไข กรุณารีเฟรช".to_string(),
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
                pos_error::AppError::Domain(msg) => {
                    V3Error::new("domainError", None, msg, StatusCode::BAD_REQUEST)
                }
                other => {
                    tracing::error!(error = %other, "branchShiftsSave failed");
                    server_error()
                }
            })?;

        Ok(BranchShiftsData::from_plan(&plan))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(BranchShiftsSaveCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::branch_shifts::export_types(dir)
}
