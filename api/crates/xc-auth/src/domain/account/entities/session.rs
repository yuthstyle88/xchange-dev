use super::employee::Employee;
use super::owner::Owner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedIn {
    Owner(Owner),
    Employee(Employee),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub token: String,
    pub who: SignedIn,
}
