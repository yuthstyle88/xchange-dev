use serde::{Deserialize, Serialize};

use super::scoped_role::ScopedRole;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub user_id: String,
    pub organization_id: String,
    pub roles: Vec<ScopedRole>,
    pub customer_id: Option<String>,
}

impl Actor {
    pub fn new(
        user_id: &str,
        organization_id: &str,
        roles: Vec<ScopedRole>,
        customer_id: Option<&str>,
    ) -> Self {
        Self {
            user_id: user_id.to_string(),
            organization_id: organization_id.to_string(),
            roles,
            customer_id: customer_id.map(str::to_string),
        }
    }
}
