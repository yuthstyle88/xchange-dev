//! The web request's services on the real database (design `kiosk-shared-phone.md` §4.3): the
//! kiosk core with today's ports and audit log, and the queue's issuer — the same pieces
//! `POST /api/kiosk` builds (`presentation/http/kiosk.rs`), for the v3 web-request commands.

use std::sync::Arc;

use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::application::kiosk::web_request::WebRequestService;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use super::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};

/// `WebRequestService` over `pool`.
pub fn web_request_service(pool: &PgPool) -> WebRequestService {
    let audit: Arc<dyn KioskAuditLog> = Arc::new(PgKioskAuditLog::with_pool(pool.clone()));
    let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
    let ports = Arc::new(PgKioskPorts::new(pool.clone(), audit, source));
    let kiosk = KioskService::new(BranchTx::new(Transactor::new(pool.clone())), ports);
    let queue: Arc<dyn QueueTickets> = Arc::new(
        TicketIssuer::new(
            BranchTx::new(Transactor::new(pool.clone())),
            Arc::new(PgQueueRepository::new(pool.clone())),
        )
        .with_booking_events(pg_booking_lifecycle(pool.clone())),
    );
    WebRequestService::new(kiosk, queue)
}
