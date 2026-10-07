//! Tests for token file permissions, configuration, and service generation.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use xc_device_agent::client::DrawerInfo;
use xc_device_agent::config::{
    ConfigFile, linux_systemd_service_content, linux_systemd_user_service_content, load_config,
    load_token, macos_launch_agent_plist, save_config, save_token,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("xc-agent-cfg-{}-{nanos}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[cfg(unix)]
#[test]
fn the_token_file_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch_dir();
    let token_path = dir.join("token");
    let secret = "xcda_0123456789abcdef0123456789abcdef";

    save_token(&token_path, secret).expect("save token");

    let loaded = load_token(&token_path).expect("load token");
    assert_eq!(loaded, secret);

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

#[test]
fn config_save_and_load_round_trip() {
    let dir = scratch_dir();
    let config_path = dir.join("config.json");

    let cfg = ConfigFile {
        server: "https://desk.example.com".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80_".to_owned(),
        },
        channel: "stable".to_owned(),
        agent_id: Some("agent-uuid-1".to_owned()),
        branch_id: Some("branch-uuid-2".to_owned()),
        agent_name: Some("Counter Mac".to_owned()),
    };

    save_config(&config_path, &cfg).expect("save config");
    let loaded = load_config(&config_path).expect("load config");

    assert_eq!(loaded.server, cfg.server);
    assert_eq!(loaded.drawer, cfg.drawer);
    assert_eq!(loaded.channel, cfg.channel);
    assert_eq!(loaded.agent_id, cfg.agent_id);
    assert_eq!(loaded.branch_id, cfg.branch_id);
    assert_eq!(loaded.agent_name, cfg.agent_name);
}

#[test]
fn macos_launch_agent_plist_format() {
    let binary = PathBuf::from("/usr/local/bin/xc-device-agent");
    let plist = macos_launch_agent_plist(&binary);

    assert!(plist.contains("<string>com.spacing.xc-device-agent</string>"));
    assert!(plist.contains("<string>/usr/local/bin/xc-device-agent</string>"));
    assert!(plist.contains("<string>run</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("<key>ThrottleInterval</key>"));
    assert!(plist.contains("<integer>30</integer>"));
}

#[test]
fn linux_systemd_service_format() {
    let binary = PathBuf::from("/usr/bin/xc-device-agent");
    let unit = linux_systemd_service_content(&binary);

    assert!(unit.contains("User=xc-agent"));
    assert!(unit.contains("Group=lp"));
    assert!(unit.contains("Environment=XC_AGENT_CONFIG_DIR=/etc/xc-device-agent"));
    assert!(unit.contains("ExecStart=/usr/bin/xc-device-agent run"));
    assert!(unit.contains("Restart=always"));
}

#[test]
fn linux_systemd_user_service_format() {
    let binary = PathBuf::from("/usr/bin/xc-device-agent");
    let config_dir = PathBuf::from("/home/user/.config/xc-device-agent");
    let unit = linux_systemd_user_service_content(&binary, &config_dir);

    assert!(!unit.contains("User="));
    assert!(!unit.contains("Group="));
    assert!(unit.contains("Environment=XC_AGENT_CONFIG_DIR=/home/user/.config/xc-device-agent"));
    assert!(unit.contains("ExecStart=/usr/bin/xc-device-agent run"));
    assert!(unit.contains("WantedBy=default.target"));
}
