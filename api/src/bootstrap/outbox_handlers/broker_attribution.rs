//! RA-2 (#980): the Broker attribution handler, registered when `Settings::broker` is set
//! (`XC_BROKER_BASE_URL` and `XC_BROKER_SERVICE_TOKEN`, design `broker-seam.md` §5.4).

use std::sync::Arc;

use xc_outbox::domain::handler::OutboxHandler;
use xc_referral::infrastructure::booking_member_keys::BookingMemberKeys;
use xc_referral::infrastructure::broker_attribution_handler::BrokerAttributionHandler;

use crate::bootstrap::AppState;

/// `None` when Broker is not configured; then the rows wait and expire after 7 days.
pub fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> {
    let broker = state.settings.broker.as_ref()?;
    Some(Arc::new(BrokerAttributionHandler::new(
        state.db.clone(),
        broker.base_url.clone(),
        broker.service_token.clone(),
        Arc::new(BookingMemberKeys),
    )))
}
