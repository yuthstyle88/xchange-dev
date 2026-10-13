//! The commission term of attributed contribution (D-15; design §6.1 "Broker-Platform commission … `awaitsBroker`").

use futures::future::BoxFuture;
use pos_error::AppResult;
use uuid::Uuid;
use xc_kernel::money::Money;

use super::unavailable::Unavailable;

/// The commission expense of the given exchanges, or why it is unavailable.
pub trait CampaignCommission: Send + Sync {
    fn commission_of<'a>(
        &'a self,
        transactions: &'a [Uuid],
    ) -> BoxFuture<'a, AppResult<Result<Money, Unavailable>>>;
}

/// Today's commission: Broker publishes no endpoint (EX-10 parked), so it is always `AwaitsBroker`.
pub struct BrokerNotConnected;

impl CampaignCommission for BrokerNotConnected {
    fn commission_of<'a>(
        &'a self,
        _transactions: &'a [Uuid],
    ) -> BoxFuture<'a, AppResult<Result<Money, Unavailable>>> {
        Box::pin(async { Ok(Err(Unavailable::AwaitsBroker)) })
    }
}
