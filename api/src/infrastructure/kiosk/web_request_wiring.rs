//! The web request's services on the real database (design `kiosk-shared-phone.md` §4.3): the
//! kiosk core with today's ports and audit log, and the queue's issuer — the same pieces
//! `POST /api/kiosk` builds (`presentation/http/kiosk.rs`), for the v3 web-request commands.

use std::sync::Arc;

use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::kiosk_queue_events::KioskQueueEvents;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::application::kiosk::queue_ended::KioskQueueEnded;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::application::kiosk::web_request::WebRequestService;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use super::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};

/// The queue's issuer, telling bookings when a ticket ends (the 7th `pg_booking_lifecycle` call site,
/// `booking_lifecycle_factory_guard_test.rs`), shared by the web request and `KioskQueueEnded`.
fn queue_tickets(pool: &PgPool) -> Arc<dyn QueueTickets> {
    Arc::new(
        TicketIssuer::new(
            BranchTx::new(Transactor::new(pool.clone())),
            Arc::new(PgQueueRepository::new(pool.clone())),
        )
        .with_booking_events(pg_booking_lifecycle(pool.clone())),
    )
}

/// `WebRequestService` over `pool`.
pub fn web_request_service(pool: &PgPool) -> WebRequestService {
    let audit: Arc<dyn KioskAuditLog> = Arc::new(PgKioskAuditLog::with_pool(pool.clone()));
    let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
    let ports = Arc::new(PgKioskPorts::new(pool.clone(), audit, source));
    let kiosk = KioskService::new(BranchTx::new(Transactor::new(pool.clone())), ports);
    let queue = queue_tickets(pool);
    WebRequestService::new(kiosk, queue)
}

/// `KioskQueueEnded` over `pool` (`kiosk-cancel.md` §3.7, XC-BE-GAPS-A row 6), for
/// `QueueDesk::with_kiosk_events`: the same audit log and ports as `web_request_service`.
pub fn kiosk_queue_events(pool: &PgPool) -> Arc<dyn KioskQueueEvents> {
    let audit: Arc<dyn KioskAuditLog> = Arc::new(PgKioskAuditLog::with_pool(pool.clone()));
    let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
    let ports = Arc::new(PgKioskPorts::new(pool.clone(), audit, source));
    let queue = queue_tickets(pool);
    Arc::new(KioskQueueEnded::new(
        PgKioskRepository::new(pool.clone()),
        ports,
        queue,
    ))
}
