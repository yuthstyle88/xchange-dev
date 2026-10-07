//! Integration test for `xc-device-agent install --no-service`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use xc_device_agent::config::{load_config, load_token};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_home_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("xc-agent-home-{}-{nanos}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch home dir");
    dir
}

fn assert_no_service_units(dir: &std::path::Path) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            assert_no_service_units(&path);
        } else {
            let name = path.file_name().unwrap().to_string_lossy();
            assert!(
                !name.ends_with(".service"),
                "found unexpected service file: {}",
                path.display()
            );
            assert!(
                !name.ends_with(".plist"),
                "found unexpected plist file: {}",
                path.display()
            );
        }
    }
}

#[test]
fn install_with_no_service_flag_writes_config_and_token_without_service_unit() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();

    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("read request line");
        assert!(
            request_line.starts_with("POST /api/device-agent/pair"),
            "expected pair request, got: {request_line}"
        );

        let mut content_length = 0;
        loop {
            let mut header_line = String::new();
            reader
                .read_line(&mut header_line)
                .expect("read header line");
            if header_line == "\r\n" || header_line == "\n" || header_line.is_empty() {
                break;
            }
            let lower = header_line.to_lowercase();
            if lower.starts_with("content-length:") {
                if let Some(val) = header_line.split(':').nth(1) {
                    content_length = val.trim().parse::<usize>().unwrap_or(0);
                }
            }
        }

        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).expect("read body");
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("PAIR-CODE-99999"),
            "body should contain pairing code"
        );

        let answer = serde_json::json!({
            "agentId": "agent-ci-test-1",
            "token": "xcda_token_secret_ci_test_12345",
            "branchId": "branch-main-1",
            "agentName": "CI Counter Runner"
        })
        .to_string();

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            answer.len(),
            answer
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        stream.flush().expect("flush stream");
    });

    let temp_home = scratch_home_dir();
    let server_url = format!("http://127.0.0.1:{port}");

    let bin = env!("CARGO_BIN_EXE_xc-device-agent");
    let output = std::process::Command::new(bin)
        .arg("install")
        .arg("--server")
        .arg(&server_url)
        .arg("--code")
        .arg("PAIR-CODE-99999")
        .arg("--cups-queue")
        .arg("POS80_RECEIPT")
        .arg("--channel")
        .arg("canary")
        .arg("--no-service")
        .env("HOME", &temp_home)
        .env_remove("XC_AGENT_CONFIG_DIR")
        .output()
        .expect("run xc-device-agent install --no-service");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "install failed with exit code {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );

    // 1. Check one line says no service was installed
    assert!(
        stdout.contains("No service unit was installed (--no-service)."),
        "stdout missing expected message: {stdout}"
    );

    // 2. Check config file was written correctly
    let config_path = temp_home
        .join(".config")
        .join("xc-device-agent")
        .join("config.json");
    assert!(
        config_path.exists(),
        "config.json should exist at {}",
        config_path.display()
    );

    let config = load_config(&config_path).expect("load config");
    assert_eq!(config.server, server_url);
    assert_eq!(config.drawer.transport, "CUPS");
    assert_eq!(config.drawer.target, "POS80_RECEIPT");
    assert_eq!(config.channel, "canary");
    assert_eq!(config.agent_id.as_deref(), Some("agent-ci-test-1"));
    assert_eq!(config.branch_id.as_deref(), Some("branch-main-1"));
    assert_eq!(config.agent_name.as_deref(), Some("CI Counter Runner"));

    // 3. Check token was written with 0600 permissions
    let token_path = temp_home
        .join(".config")
        .join("xc-device-agent")
        .join("token");
    assert!(
        token_path.exists(),
        "token should exist at {}",
        token_path.display()
    );

    let token = load_token(&token_path).expect("load token");
    assert_eq!(token, "xcda_token_secret_ci_test_12345");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token_path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "token file mode must be 0600, got {mode:o}"
        );
    }

    // 4. Check no unit file was written under temp HOME
    assert_no_service_units(&temp_home);
    assert!(!temp_home.join(".config").join("systemd").exists());
    assert!(!temp_home.join("Library").join("LaunchAgents").exists());

    server_thread.join().expect("server thread finished");
}
