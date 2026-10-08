//! Configuration, token storage, and service management for xc-device-agent.

#[cfg(not(unix))]
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};

use crate::client::DrawerInfo;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("XC_AGENT_BUILD");
pub const BUILT_AT: &str = env!("XC_AGENT_BUILT_AT");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    pub server: String,
    pub drawer: DrawerInfo,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub agent_id: Option<String>,
    pub branch_id: Option<String>,
    pub agent_name: Option<String>,
}

fn default_channel() -> String {
    "stable".to_string()
}

pub fn is_root() -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("Uid:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() > 1 && parts[1] == "0" {
                        return true;
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    {
        if let Ok(u) = std::env::var("USER") {
            if u == "root" {
                return true;
            }
        }
    }
    false
}

pub fn default_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XC_AGENT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "linux")]
    {
        if is_root() {
            return PathBuf::from("/etc/xc-device-agent");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("xc-device-agent");
    }
    PathBuf::from("/etc/xc-device-agent")
}

pub fn current_os() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn current_host() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "localhost".to_string())
}

#[cfg(target_os = "linux")]
fn get_user_uid_gid(username: &str) -> Option<(u32, u32)> {
    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 4 && parts[0] == username {
                if let (Ok(uid), Ok(gid)) = (parts[2].parse::<u32>(), parts[3].parse::<u32>()) {
                    return Some((uid, gid));
                }
            }
        }
    }
    if let Ok(out) = std::process::Command::new("id")
        .arg("-u")
        .arg(username)
        .output()
    {
        if out.status.success() {
            if let Ok(s) = std::str::from_utf8(&out.stdout) {
                if let Ok(uid) = s.trim().parse::<u32>() {
                    let gid = std::process::Command::new("id")
                        .arg("-g")
                        .arg(username)
                        .output()
                        .ok()
                        .and_then(|o| {
                            std::str::from_utf8(&o.stdout)
                                .ok()
                                .and_then(|s| s.trim().parse::<u32>().ok())
                        })
                        .unwrap_or(uid);
                    return Some((uid, gid));
                }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub fn service_user_exists(username: &str) -> bool {
    get_user_uid_gid(username).is_some()
}

pub fn chown_to_service_user(path: &Path, username: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        if let Some((uid, gid)) = get_user_uid_gid(username) {
            let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
        }
    }
    let _ = (path, username);
    Ok(())
}

/// Saves the token file with mode 0600 (readable by owner only).
pub fn save_token(path: &Path, token: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(token.as_bytes())?;
        file.sync_data()?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        #[cfg(target_os = "linux")]
        {
            let _ = chown_to_service_user(path, "xc-agent");
        }
    }
    #[cfg(not(unix))]
    {
        let mut file = File::create(path)?;
        file.write_all(token.as_bytes())?;
        file.sync_data()?;
    }
    Ok(())
}

pub fn load_token(path: &Path) -> io::Result<String> {
    let content = std::fs::read_to_string(path)?;
    Ok(content.trim().to_string())
}

pub fn save_config(path: &Path, config: &ConfigFile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, data)
}

pub fn load_config(path: &Path) -> io::Result<ConfigFile> {
    let data = std::fs::read_to_string(path)?;
    serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn macos_launch_agent_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("Library/LaunchAgents/com.spacing.xc-device-agent.plist"))
}

pub fn macos_launch_agent_plist(binary_path: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.spacing.xc-device-agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>30</integer>
</dict>
</plist>
"#,
        binary_path.display()
    )
}

pub fn linux_systemd_service_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system/xc-device-agent.service")
}

pub fn linux_systemd_service_content(binary_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=XCHANGE Counter Device Agent
After=network.target

[Service]
Type=simple
User=xc-agent
Group=lp
Environment=XC_AGENT_CONFIG_DIR=/etc/xc-device-agent
ExecStart={} run
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
"#,
        binary_path.display()
    )
}

pub fn linux_systemd_user_service_content(binary_path: &Path, config_dir: &Path) -> String {
    format!(
        r#"[Unit]
Description=XCHANGE Counter Device Agent
After=network.target

[Service]
Type=simple
Environment=XC_AGENT_CONFIG_DIR={}
ExecStart={} run
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
"#,
        config_dir.display(),
        binary_path.display()
    )
}

pub fn install_service(binary_path: &Path) -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Some(plist_path) = macos_launch_agent_path() {
            if let Some(parent) = plist_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&plist_path, macos_launch_agent_plist(binary_path))?;
            return Ok(plist_path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if !service_user_exists("xc-agent") {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "service user 'xc-agent' does not exist; create it with: useradd -r -s /usr/sbin/nologin xc-agent",
            ));
        }

        let service_path = linux_systemd_service_path();
        if let Some(parent) = service_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&service_path, linux_systemd_service_content(binary_path)).is_ok() {
            return Ok(service_path);
        }
        // Fallback to user systemd unit if non-root
        if let Ok(home) = std::env::var("HOME") {
            let config_dir = default_config_dir();
            let user_service = PathBuf::from(home)
                .join(".config")
                .join("systemd")
                .join("user")
                .join("xc-device-agent.service");
            if let Some(parent) = user_service.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &user_service,
                linux_systemd_user_service_content(binary_path, &config_dir),
            )?;
            return Ok(user_service);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "service installation not supported on this platform",
    ))
}

pub fn uninstall_service() -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        if let Some(plist_path) = macos_launch_agent_path() {
            if plist_path.exists() {
                std::fs::remove_file(plist_path)?;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let service_path = linux_systemd_service_path();
        if service_path.exists() {
            let _ = std::fs::remove_file(&service_path);
        }
        if let Ok(home) = std::env::var("HOME") {
            let user_service = PathBuf::from(home)
                .join(".config")
                .join("systemd")
                .join("user")
                .join("xc-device-agent.service");
            if user_service.exists() {
                let _ = std::fs::remove_file(&user_service);
            }
        }
    }

    Ok(())
}
