//! A printer on LAN through raw TCP write to host:9100 (§5.4c *What the agent drives*).

use crate::transport::{DrawerTransport, SendReceipt, TransportError};

/// A network printer reached by raw TCP on port 9100 (or another port).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpTransport9100 {
    pub host: String,
    pub port: u16,
}

impl TcpTransport9100 {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    pub fn with_default_port(host: impl Into<String>) -> Self {
        Self::new(host, 9100)
    }
}

impl DrawerTransport for TcpTransport9100 {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        pos_printer::network_printer::send(&self.host, self.port, bytes)
            .await
            .map_err(|e| TransportError::Failed(e.to_string()))?;
        Ok(SendReceipt { cups_job: None })
    }
}
