//! `TransferEnds`: dynamic trait defining ends, validations, and postings per transfer kind.

use xc_auth::domain::access::Actor;

use crate::domain::posting::PostingRequest;
use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, TransferError, TransferKind,
};

pub trait TransferEnds: Send + Sync + 'static {
    /// The transfer kind this implementation handles.
    fn kind(&self) -> TransferKind;

    /// Validates ends and who may create this transfer.
    fn check_create(&self, who: &Actor, t: &NewTransfer) -> Result<(), TransferError>;

    /// Validates who may receive this transfer.
    fn check_receive(&self, who: &Actor, t: &CashTransfer) -> Result<(), TransferError>;

    /// Generates posting requests at DISPATCHED state.
    fn on_dispatch(&self, t: &CashTransfer) -> Vec<PostingRequest>;

    /// Generates posting requests at RECEIVED state.
    fn on_receive(&self, t: &CashTransfer) -> Vec<PostingRequest>;
}
