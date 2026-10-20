//! `UserProfile` (design §4, the four new `users` columns): the employee fields an ADMIN saves with
//! `userProfileSave` (F3, through `UserService::save_profile`, C1c). Every field is optional; an empty
//! string is stored as NULL. `employee_code` is unique per organisation, case-insensitive (§4).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub employee_code: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub tell_code: Option<String>,
}
