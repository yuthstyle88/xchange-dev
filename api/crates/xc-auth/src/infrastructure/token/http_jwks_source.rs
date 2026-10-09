use std::time::Duration;

use futures::future::BoxFuture;

use crate::domain::token::{Jwks, JwksSource, TokenError};

/// Fetches JSON Web Key Sets (JWKS) from an HTTP URL.
#[derive(Clone)]
pub struct HttpJwksSource {
    client: reqwest::Client,
    url: String,
}

impl HttpJwksSource {
    /// Creates a new `HttpJwksSource` targeting the given JWKS URL.
    pub fn new(url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            url: url.into(),
        }
    }

    /// Creates a new `HttpJwksSource` with a custom HTTP client and URL.
    pub fn with_client(client: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }
}

impl JwksSource for HttpJwksSource {
    fn fetch(&self) -> BoxFuture<'_, Result<Jwks, TokenError>> {
        Box::pin(async move {
            let res = self
                .client
                .get(&self.url)
                .send()
                .await
                .map_err(|_| TokenError::KeysUnavailable)?;

            if !res.status().is_success() {
                return Err(TokenError::KeysUnavailable);
            }

            let jwks: Jwks = res.json().await.map_err(|_| TokenError::KeysUnavailable)?;
            Ok(jwks)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[tokio::test]
    async fn fetches_jwks_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = r#"{"keys":[{"kid":"k1","kty":"OKP","crv":"Ed25519","x":"abc"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let source = HttpJwksSource::new(format!("http://127.0.0.1:{port}/jwks.json"));
        let jwks = source.fetch().await.unwrap();
        assert_eq!(jwks.keys.len(), 1);
        assert_eq!(jwks.keys[0].kid, "k1");
    }

    #[tokio::test]
    async fn unreachable_url_returns_keys_unavailable() {
        let source = HttpJwksSource::new("http://127.0.0.1:1/jwks.json");
        let err = source.fetch().await.unwrap_err();
        assert_eq!(err, TokenError::KeysUnavailable);
    }
}
