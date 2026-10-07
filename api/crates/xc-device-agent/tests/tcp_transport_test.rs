//! Tests for TcpTransport9100 against a local TcpListener.

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use xc_device_agent::tcp_transport::TcpTransport9100;
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

const KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];

#[tokio::test]
async fn tcp_transport_sends_exact_bytes_to_listener() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    let port = addr.port();

    let server_task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = Vec::new();
        socket.read_to_end(&mut buf).await.expect("read to end");
        buf
    });

    let transport = TcpTransport9100::new("127.0.0.1", port);
    let receipt = transport.send(&KICK).await.expect("send bytes");

    assert_eq!(receipt, SendReceipt { cups_job: None });

    let received = server_task.await.expect("server task");
    assert_eq!(received, KICK);
}

#[tokio::test]
async fn tcp_transport_fails_when_cannot_connect() {
    // Port 1 should not have an open listener
    let transport = TcpTransport9100::new("127.0.0.1", 1);
    let result = transport.send(&KICK).await;

    assert!(matches!(result, Err(TransportError::Failed(_))));
}
