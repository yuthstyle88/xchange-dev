use sqlx::PgPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerHost(String);

impl OwnerHost {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let host = raw.trim().to_ascii_lowercase();
        if host.is_empty()
            || host.chars().any(char::is_whitespace)
            || host.contains('/')
            || host.contains('@')
            || host.ends_with(":80")
            || host.ends_with(":443")
        {
            return Err(format!("host: invalid host {raw:?}"));
        }
        Ok(Self(host))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub async fn record(&self, pool: &PgPool, org_id: &str) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO organization_hosts (host, organization_id) \
             VALUES ($1, $2::uuid) \
             ON CONFLICT (host) DO NOTHING",
        )
        .bind(self.as_str())
        .bind(org_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        let existing_org_id: String = sqlx::query_scalar(
            "SELECT organization_id::text FROM organization_hosts WHERE host = $1",
        )
        .bind(self.as_str())
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

        if existing_org_id != org_id {
            return Err(format!(
                "host: {} belongs to another organisation",
                self.as_str()
            ));
        }

        Ok(())
    }
}
