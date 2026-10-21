//! Booking commands runner and registration (§6, §4.4 BK-6 #1388).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use actix_web::web;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;

use xc_booking::application::booking_service::BookingService;
use xc_booking::application::check_in::CheckInService;
use xc_booking::application::slot_service::SlotService;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::application::customer::contact_reserve::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::command::{CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[path = "bookings/cancel.rs"]
pub mod cancel;
#[path = "bookings/check_in.rs"]
pub mod check_in;
#[path = "bookings/closure_add.rs"]
pub mod closure_add;
#[path = "bookings/closure_remove.rs"]
pub mod closure_remove;
#[path = "bookings/confirm.rs"]
pub mod confirm;
#[path = "bookings/create.rs"]
pub mod create;
#[path = "bookings/kiosk_check_in.rs"]
pub mod kiosk_check_in;
#[path = "bookings/policy.rs"]
pub mod policy;
#[path = "bookings/qr.rs"]
pub mod qr;
#[path = "bookings/reschedule.rs"]
pub mod reschedule;
#[path = "bookings/slot_capacity.rs"]
pub mod slot_capacity;
#[path = "bookings/slot_close.rs"]
pub mod slot_close;
#[path = "bookings/slot_open.rs"]
pub mod slot_open;
#[path = "bookings/slot_rule.rs"]
pub mod slot_rule;

pub use closure_remove::{ClosureRemove, ClosureRemoveInput};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingOkOutput {
    pub ok: bool,
}

#[derive(Clone)]
pub struct BuiltBookingServices {
    pub booking_service: Arc<BookingService>,
    pub check_in_service: Arc<CheckInService>,
    pub slot_service: Arc<SlotService>,
}

#[derive(Clone)]
pub struct BookingServices {
    pub pool: PgPool,
    pub branch_tx: BranchTx,
    services: Arc<OnceLock<BuiltBookingServices>>,
}

impl BookingServices {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        Self {
            pool,
            branch_tx,
            services: Arc::new(OnceLock::new()),
        }
    }

    pub(super) fn services(&self) -> Result<&BuiltBookingServices, V3Error> {
        if let Some(services) = self.services.get() {
            return Ok(services);
        }
        let installed_keys = xc_customer::get_pii_keys().map_err(|_| server_error())?;
        let pii_keys1 = installed_keys.as_ref().clone();
        let pii_keys2 = installed_keys.as_ref().clone();

        let booking_repo = Arc::new(PgBookingRepository::new(self.pool.clone()));
        let slot_repo = Arc::new(PgSlotRepository::new(self.pool.clone()));
        let queue_repo = Arc::new(PgQueueRepository::new(self.pool.clone()));

        let booking_events: Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents> =
            pg_booking_lifecycle(self.pool.clone());

        let issuer = Arc::new(
            TicketIssuer::new(self.branch_tx.clone(), queue_repo)
                .with_booking_events(booking_events),
        );

        let contact_reserve = Arc::new(ContactReserve::new(self.branch_tx.clone()));

        let booking_service = Arc::new(
            BookingService::new(
                self.pool.clone(),
                booking_repo.clone(),
                slot_repo.clone(),
                contact_reserve,
                Some(Arc::new(
                    crate::presentation::http::v3::attach_referral::capture_service_for_pool(
                        self.pool.clone(),
                    ),
                )),
                pii_keys1,
            )
            .with_queue(issuer.clone()),
        );

        let check_in_service = Arc::new(CheckInService::new(
            self.pool.clone(),
            self.branch_tx.clone(),
            booking_repo,
            slot_repo.clone(),
            issuer,
            pii_keys2,
        ));

        let slot_service = Arc::new(SlotService::new(self.pool.clone(), slot_repo));

        let _ = self.services.set(BuiltBookingServices {
            booking_service,
            check_in_service,
            slot_service,
        });
        Ok(self.services.get().expect("services initialized"))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let services = Arc::new(BookingServices::new(state.db.clone()));
    CommandEndpoint::register(cfg, Arc::new(create::BookingCreate::new(services.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(confirm::BookingConfirm::new(services.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(cancel::BookingCancel::new(services.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(reschedule::BookingReschedule::new(services.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(qr::BookingQrReissue::new(services.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(check_in::BookingCheckIn::new(services.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(kiosk_check_in::KioskBookingCheckIn::new(services.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(slot_rule::SlotRuleSet::new(services.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(closure_add::ClosureAdd::new(services.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(closure_remove::ClosureRemove::new(services.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(slot_capacity::SlotCapacitySet::new(services.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(slot_close::SlotClose::new(services.clone())));
    CommandEndpoint::register(cfg, Arc::new(slot_open::SlotOpen::new(services.clone())));
    CommandEndpoint::register(cfg, Arc::new(policy::BookingPolicySet::new(services)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    create::BookingCreateInput::export_all(&cfg)?;
    create::BookingCreateOutput::export_all(&cfg)?;
    confirm::BookingConfirmInput::export_all(&cfg)?;
    confirm::BookingConfirmOutput::export_all(&cfg)?;
    cancel::BookingCancelInput::export_all(&cfg)?;
    cancel::BookingCancelOutput::export_all(&cfg)?;
    reschedule::BookingRescheduleInput::export_all(&cfg)?;
    reschedule::BookingRescheduleOutput::export_all(&cfg)?;
    qr::BookingQrReissueInput::export_all(&cfg)?;
    qr::BookingQrReissueOutput::export_all(&cfg)?;
    check_in::BookingCheckInInput::export_all(&cfg)?;
    check_in::BookingCheckInOutput::export_all(&cfg)?;
    kiosk_check_in::KioskBookingCheckInInput::export_all(&cfg)?;
    kiosk_check_in::KioskBookingCheckInOutput::export_all(&cfg)?;
    slot_rule::SlotRuleSetInput::export_all(&cfg)?;
    closure_add::ClosureAddInput::export_all(&cfg)?;
    closure_remove::ClosureRemoveInput::export_all(&cfg)?;
    slot_capacity::SlotCapacitySetInput::export_all(&cfg)?;
    slot_capacity::SlotCapacitySetOutput::export_all(&cfg)?;
    slot_close::SlotCloseInput::export_all(&cfg)?;
    slot_close::SlotCloseOutput::export_all(&cfg)?;
    slot_open::SlotOpenInput::export_all(&cfg)?;
    slot_open::SlotOpenOutput::export_all(&cfg)?;
    policy::BookingPolicySetInput::export_all(&cfg)?;
    BookingOkOutput::export_all(&cfg)?;
    Ok(())
}
