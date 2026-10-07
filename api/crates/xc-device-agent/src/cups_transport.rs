//! A printer on USB through a CUPS raw queue (§5.4c *What the agent drives*): `lpstat -o
//! <queue>` must list no pending job, then `lp -d <queue> -o raw` with the bytes on stdin.
//! Both are run directly, never through a shell.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::transport::{DrawerTransport, SendReceipt, TransportError};

pub struct CupsTransport {
    pub queue: String,
    pub lp: PathBuf,
    pub lpstat: PathBuf,
}

impl CupsTransport {
    /// `lp` and `lpstat` found on `PATH`.
    pub fn new(queue: &str) -> Self {
        Self::with_commands(queue, PathBuf::from("lp"), PathBuf::from("lpstat"))
    }

    /// The two commands named by path (tests put fakes there).
    pub fn with_commands(queue: &str, lp: PathBuf, lpstat: PathBuf) -> Self {
        Self {
            queue: queue.to_owned(),
            lp,
            lpstat,
        }
    }
}

fn parse_cups_job(stdout: &str) -> Option<String> {
    if let Some(idx) = stdout.find("request id is ") {
        let after = &stdout[idx + "request id is ".len()..];
        if let Some(token) = after.split_whitespace().next() {
            let cleaned = token.trim_matches(|c: char| c == '(' || c == ')' || c == ',');
            if !cleaned.is_empty() {
                return Some(cleaned.to_owned());
            }
        }
    }
    None
}

impl DrawerTransport for CupsTransport {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        // 1. Check pending jobs with `lpstat -o <queue>`
        let lpstat_res = timeout(Duration::from_secs(5), async {
            for _ in 0..20 {
                match Command::new(&self.lpstat)
                    .args(["-o", &self.queue])
                    .output()
                    .await
                {
                    Ok(output) => return Ok(output),
                    Err(e) if e.raw_os_error() == Some(26) => {
                        // ETXTBSY: temporary race between fork and exec in multi-threaded tests
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            Command::new(&self.lpstat)
                .args(["-o", &self.queue])
                .output()
                .await
        })
        .await;

        let output = match lpstat_res {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(TransportError::Failed(e.to_string())),
            Err(_) => return Err(TransportError::Failed("lpstat timed out".to_owned())),
        };

        if !output.status.success() {
            let mut msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if msg.is_empty() {
                msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
            if msg.is_empty() {
                msg = format!("lpstat exited with status {}", output.status);
            }
            return Err(TransportError::Failed(msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let has_pending = stdout.lines().any(|line| !line.trim().is_empty());
        if has_pending {
            return Err(TransportError::QueueBusy);
        }

        // 2. Queue is idle: run `lp -d <queue> -o raw` with bytes on stdin
        let mut child = {
            let mut spawn_res = None;
            for _ in 0..20 {
                let mut cmd = Command::new(&self.lp);
                cmd.args(["-d", &self.queue, "-o", "raw"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                match cmd.spawn() {
                    Ok(child) => {
                        spawn_res = Some(Ok(child));
                        break;
                    }
                    Err(e) if e.raw_os_error() == Some(26) => {
                        // ETXTBSY: retry
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    Err(e) => {
                        spawn_res = Some(Err(e));
                        break;
                    }
                }
            }
            match spawn_res.unwrap_or_else(|| {
                let mut cmd = Command::new(&self.lp);
                cmd.args(["-d", &self.queue, "-o", "raw"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                cmd.spawn()
            }) {
                Ok(child) => child,
                Err(e) => return Err(TransportError::Failed(e.to_string())),
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(bytes).await {
                return Err(TransportError::Failed(e.to_string()));
            }
        }

        let output = match timeout(Duration::from_secs(5), child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(TransportError::Failed(e.to_string())),
            Err(_) => return Err(TransportError::Failed("lp timed out".to_owned())),
        };

        if !output.status.success() {
            let mut msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if msg.is_empty() {
                msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
            if msg.is_empty() {
                msg = format!("lp exited with status {}", output.status);
            }
            return Err(TransportError::Failed(msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let cups_job = parse_cups_job(&stdout);
        Ok(SendReceipt { cups_job })
    }
}
