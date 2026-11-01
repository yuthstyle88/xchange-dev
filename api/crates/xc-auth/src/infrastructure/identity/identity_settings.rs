#[derive(Clone, serde::Deserialize)]
pub struct IdentitySettings {
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub audience: String,
    pub issuer: String,
    pub api_key: Option<String>,
    /// Identity's `INTERNAL_SERVICE_TOKEN`, sent as `X-Internal-Token` on its admin routes (design
    /// `user-management-rbac.md` §13). `None` = admin calls answer `identityUnavailable`.
    pub internal_token: Option<String>,
}

impl std::fmt::Debug for IdentitySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentitySettings")
            .field("base_url", &self.base_url)
            .field("client_id", &self.client_id)
            .field("audience", &self.audience)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}
