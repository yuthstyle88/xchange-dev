//! Retention domain types (design `customer-identity-consent.md` §4.1).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "IDENTITY_VALUE")]
    IdentityValue,
    #[serde(rename = "CONTACT")]
    Contact,
    #[serde(rename = "CLOSED_CUSTOMER")]
    ClosedCustomer,
    #[serde(rename = "RATE_ALERT")]
    RateAlert,
    #[serde(rename = "ASSISTANT_CONVERSATION")]
    AssistantConversation,
    #[serde(rename = "PLANNER_DRAFT")]
    PlannerDraft,
}

impl DataType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataType::IdentityValue => "IDENTITY_VALUE",
            DataType::Contact => "CONTACT",
            DataType::ClosedCustomer => "CLOSED_CUSTOMER",
            DataType::RateAlert => "RATE_ALERT",
            DataType::AssistantConversation => "ASSISTANT_CONVERSATION",
            DataType::PlannerDraft => "PLANNER_DRAFT",
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "IDENTITY_VALUE" => Ok(DataType::IdentityValue),
            "CONTACT" => Ok(DataType::Contact),
            "CLOSED_CUSTOMER" => Ok(DataType::ClosedCustomer),
            "RATE_ALERT" => Ok(DataType::RateAlert),
            "ASSISTANT_CONVERSATION" => Ok(DataType::AssistantConversation),
            "PLANNER_DRAFT" => Ok(DataType::PlannerDraft),
            _ => Err(format!("unknown data type: {s}")),
        }
    }
}
