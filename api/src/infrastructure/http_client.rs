//! Shared `reqwest::Client` builder that fails loudly instead of falling back to a no-timeout `Client::new()`.

use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_USER_AGENT: &str = concat!("xchange/", env!("CARGO_PKG_VERSION"));

fn try_build_client(
    timeout: Duration,
    user_agent: &str,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .user_agent(user_agent)
        .build()
}

pub fn build_client(timeout: Duration, user_agent: &str) -> reqwest::Client {
    try_build_client(timeout, user_agent).expect("reqwest client must build (TLS backend)")
}

pub fn probe() -> anyhow::Result<()> {
    try_build_client(PROBE_TIMEOUT, PROBE_USER_AGENT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_client_returns_a_client() {
        let _client = build_client(Duration::from_secs(10), "xchange-test/1.0");
    }

    #[test]
    fn probe_is_ok() {
        assert!(probe().is_ok());
    }
}
