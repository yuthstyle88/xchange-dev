//! Stale policy domain types and evaluation.

use xc_kernel::money::Money;

use crate::domain::publication::published_rates::PublishedSide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnStale {
    Stop,
    Limit,
}

impl OnStale {
    pub fn parse_db(s: &str) -> Option<Self> {
        match s {
            "STOP" => Some(Self::Stop),
            "LIMIT" => Some(Self::Limit),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stop => "STOP",
            Self::Limit => "LIMIT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateUnavailable(pub &'static str);

#[derive(Debug, Clone)]
pub struct StalePolicy {
    pub on_stale: Option<OnStale>,
    pub stale_limit_thb: Option<Money>,
}

impl StalePolicy {
    pub fn new(on_stale: Option<OnStale>, stale_limit_thb: Option<Money>) -> Self {
        Self {
            on_stale,
            stale_limit_thb,
        }
    }

    pub fn admits(&self, side: &PublishedSide, thb: Money) -> Result<(), RateUnavailable> {
        if !side.stale {
            return Ok(());
        }
        match self.on_stale {
            Some(OnStale::Stop) => Err(RateUnavailable("stale quote refused by STOP policy")),
            Some(OnStale::Limit) => {
                if let Some(limit) = self.stale_limit_thb {
                    if thb <= limit {
                        Ok(())
                    } else {
                        Err(RateUnavailable("stale quote amount exceeds limit"))
                    }
                } else {
                    Err(RateUnavailable(
                        "stale quote refused by LIMIT policy without limit",
                    ))
                }
            }
            None => Ok(()),
        }
    }
}
