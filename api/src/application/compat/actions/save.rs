//! The save family: `saveTransaction`, the save of an exchange.
//!
//! Wired (ticket P2-S): [`SaveAction::handles`] answers `saveTransaction` on
//! both routes (`/api/rpc` and `/api/staff/rpc`).
//!
//! What the arm is made of:
//! - The payload read into `SaveTransaction`, `SaveExtras`, and `SaveActor`
//! - Registered steps in v2 order: `DeviceSaveStep`, `KioskSaveStep`, `CustodySaveStep`
//! - Execution via `SaveTransactionWorkflow::save_as`
//! - Answer formatted with v2's nine keys in fixed order

use std::sync::Arc;

use serde_json::{Map, Value};
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_shift::application::custody::service::CustodyService;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::RpcCaller;
use crate::application::compat::v2_json::V2Json;
use crate::application::settings::PaymentSettingsService;
use crate::application::workflows::booking_save_step::BookingSaveStep;
use crate::application::workflows::broker_feed_save_step::BrokerFeedSaveStep;
use crate::application::workflows::custody_save_step::CustodySaveStep;
use crate::application::workflows::device_save_step::DeviceSaveStep;
use crate::application::workflows::kiosk_save_step::KioskSaveStep;
use crate::application::workflows::payment_save_step::PaymentSaveStep;
use crate::application::workflows::save_step::{BookingClaim, SaveActor, SaveExtras, SaveStep};
use crate::application::workflows::save_transaction::SaveTransactionWorkflow;
use crate::infrastructure::devices::device_service::DeviceService;

/// The save family: `saveTransaction`, the save of an exchange.
#[derive(Clone)]
pub struct SaveAction {
    workflow: SaveTransactionWorkflow,
    steps: Vec<Arc<dyn SaveStep>>,
}

impl SaveAction {
    /// `workflow` saves an exchange; `devices`, `kiosk`, and `custody` run their
    /// steps before and after the save inside its transaction in v2's order.
    pub fn new(
        workflow: SaveTransactionWorkflow,
        devices: DeviceService,
        kiosk: KioskService,
        custody: CustodyService,
    ) -> Self {
        let steps: Vec<Arc<dyn SaveStep>> = vec![
            Arc::new(DeviceSaveStep::new(devices)),
            Arc::new(KioskSaveStep::new(kiosk)),
            Arc::new(CustodySaveStep::new(custody)),
        ];
        let workflow = workflow.with_steps(steps.clone());
        Self { workflow, steps }
    }

    /// Appends the payment save step to enforce payment channel rules on first LIVE saves.
    pub fn with_payment_check(mut self, payments: PaymentSettingsService) -> Self {
        self.steps.push(Arc::new(PaymentSaveStep::new(payments)));
        self.workflow = self.workflow.with_steps(self.steps.clone());
        self
    }

    /// Appends the booking save step to redeem and use bookings at the desk (T15-h).
    pub fn with_booking_check(mut self, lifecycle: Arc<BookingLifecycle>) -> Self {
        self.steps.push(Arc::new(BookingSaveStep::new(lifecycle)));
        self.workflow = self.workflow.with_steps(self.steps.clone());
        self
    }

    /// Appends the Broker exchange feed step (FD-2 #973). It must come after the booking step,
    /// whose `after_in` writes `bookings.transaction_ref`, which the member lookup reads.
    pub fn with_broker_feed(mut self) -> Self {
        self.steps.push(Arc::new(BrokerFeedSaveStep::new()));
        self.workflow = self.workflow.with_steps(self.steps.clone());
        self
    }
}

impl RpcAction for SaveAction {
    fn handles(&self, _route: RpcRoute, action: &str) -> bool {
        action == "saveTransaction"
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        Box::pin(async move {
            let cmd = parse_command(&call.payload);
            let extras = parse_extras(&call.payload);
            let booking = parse_booking(&call.payload);
            let actor = save_actor(call.caller);

            let saved = self
                .workflow
                .save_as_booked(call.branch, cmd, &extras, booking, &actor, call.now)
                .await?;

            let number = &saved.document_ref;
            let folder_url = format!("/documents?branch={}&ref={number}", call.branch);
            let sheet_url = format!("/?branch={}#transactions", call.branch);

            let answer = V2Json::object(vec![
                ("saved", V2Json::Bool(true)),
                ("mode", V2Json::text(saved.mode.as_str())),
                ("documentRef", V2Json::text(&saved.document_ref)),
                ("receiptNo", V2Json::text(&saved.document_ref)),
                ("passportUrl", V2Json::text(&saved.passport_url)),
                ("slipUrl", V2Json::text(&saved.slip_url)),
                ("receiptUrl", V2Json::text(&saved.receipt_url)),
                ("folderUrl", V2Json::text(folder_url)),
                ("sheetUrl", V2Json::text(sheet_url)),
            ]);

            Ok(answer)
        })
    }
}

fn save_actor(caller: &RpcCaller) -> SaveActor {
    let id = caller.actor.id().to_string();
    let (employee_id, standing) = match caller.actor.kind() {
        ActorKind::Staff => (
            Some(id.clone()),
            xc_auth::domain::access::StandingKind::Staff(
                xc_auth::domain::access::v3_roles::blueprint_roles(caller.actor.role())[0],
            ),
        ),
        ActorKind::Owner => (None, xc_auth::domain::access::StandingKind::Owner),
    };
    SaveActor {
        id,
        username: caller.username.clone(),
        employee_id,
        standing,
    }
}

fn text_field(payload: &Map<String, Value>, key: &str) -> String {
    match payload.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

fn file_key(payload: &Map<String, Value>, field: &str) -> Option<String> {
    let value = payload.get(field)?;
    match value {
        Value::Null => None,
        Value::Bool(false) => None,
        Value::String(s) if s.is_empty() => None,
        Value::Number(n) if n.as_i64() == Some(0) || n.as_f64() == Some(0.0) => None,
        Value::Object(map) => match map.get("uploadKey") {
            Some(Value::String(key)) => Some(key.clone()),
            _ => Some(String::new()),
        },
        _ => Some(String::new()),
    }
}

fn parse_extras(payload: &Map<String, Value>) -> SaveExtras {
    let staff_draft_id = payload.get("staffDraftId").and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let kiosk_request_id = payload.get("kioskRequestId").and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let count_confirmed = matches!(payload.get("countConfirmed"), Some(Value::Bool(true)));
    SaveExtras {
        staff_draft_id,
        count_confirmed,
        kiosk_request_id,
    }
}

fn parse_booking(payload: &Map<String, Value>) -> Option<BookingClaim> {
    let booking_id = match payload.get("onlineBookingId") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let customer_confirmed = matches!(
        payload.get("onlineBookingCustomerConfirmed"),
        Some(Value::Bool(true))
    );
    Some(BookingClaim {
        booking_id,
        customer_confirmed,
    })
}

fn parse_command(payload: &Map<String, Value>) -> SaveTransaction {
    let customer = ReserveCustomer {
        name: text_field(payload, "name"),
        passport_no: text_field(payload, "passportNo"),
        nationality: text_field(payload, "nationality"),
        birth_date: text_field(payload, "birthDate"),
        passport_expiry: text_field(payload, "passportExpiry"),
        mode: text_field(payload, "mode"),
    };
    let reference_rate = payload
        .get("referenceRate")
        .filter(|v| !v.is_null())
        .map(|_| text_field(payload, "referenceRate"));

    SaveTransaction {
        customer,
        submission_id: text_field(payload, "submissionId"),
        customer_id: text_field(payload, "customerId"),
        side: text_field(payload, "type"),
        currency: text_field(payload, "currency"),
        amount: text_field(payload, "amount"),
        rate: text_field(payload, "rate"),
        fee: text_field(payload, "fee"),
        bank_fee: text_field(payload, "bankFee"),
        method: text_field(payload, "method"),
        payment_ref: text_field(payload, "ref"),
        account: text_field(payload, "account"),
        staff: text_field(payload, "staff"),
        booking: text_field(payload, "booking"),
        notes: text_field(payload, "notes"),
        value_date: text_field(payload, "valueDate"),
        paid_at: text_field(payload, "paidAt"),
        payment_confirmed: matches!(payload.get("paymentConfirmed"), Some(Value::Bool(true))),
        identity_confirmed: matches!(payload.get("identityConfirmed"), Some(Value::Bool(true))),
        passport: file_key(payload, "passport"),
        slip: file_key(payload, "slip"),
        reference_rate,
    }
}
