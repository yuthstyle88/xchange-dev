//! Temporary password (design §3.4, V.9 :90-95): 16 characters, letters and digits only, never `0 O o 1 l I`
//! (ruling #564 issuecomment-6246076019 Q3). No Display, Serialize or Clone.

pub struct TempPassword(String);

impl TempPassword {
    /// Unbiased (rejection sampling) from `ring::rand::SystemRandom`.
    pub fn generate() -> TempPassword {
        todo!("RBAC function ticket: TempPassword::generate (§3.4)")
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for TempPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TempPassword(..)")
    }
}
