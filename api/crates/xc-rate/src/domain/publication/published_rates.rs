//! Published rates port and value objects.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side, UnitSize};

use crate::domain::publication::publication::{Publication, PublicationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationRef {
    pub id: PublicationId,
    pub version: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PublishedRateLineId(Uuid);

impl PublishedRateLineId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PublishedRateLineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PublishedRateLineId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedSide {
    pub line_id: PublishedRateLineId,
    pub rate: Rate,
    pub unit_size: UnitSize,
    pub publication: PublicationRef,
    pub stale: bool,
    pub source_date: Option<chrono::NaiveDate>,
}

pub trait PublishedRates: Send + Sync {
    /// The branch's current publication and its lines; None before the first publication.
    fn current<'a>(&'a self, branch: &'a BranchId)
    -> BoxFuture<'a, AppResult<Option<Publication>>>;

    /// One line of the current publication for a side, in the caller's transaction.
    fn line_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        code: CurrencyCode,
        side: Side,
    ) -> BoxFuture<'a, AppResult<Option<PublishedSide>>>;
}
