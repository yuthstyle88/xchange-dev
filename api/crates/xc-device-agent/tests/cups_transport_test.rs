//! Row AG-5 (`docs/design/xchange-v3.md` §5.4c *What the agent drives*): the CUPS raw queue
//! transport against a fake `lp` and `lpstat`. Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own. Unix only (the fakes are shell
//! scripts; the agent's targets are macOS and Linux).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use xc_device_agent::cups_transport::CupsTransport;
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

const KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory with a fake `lpstat` that prints `lpstat_out` and a fake `lp` that exits
/// `lp_exit` after printing `lp_out` (stdout) or `lp_err` (stderr). Each writes its arguments,
/// one per line, to `<name>.args`; `lp` writes its stdin to `lp.stdin`.
struct Fakes {
    dir: PathBuf,
}

impl Fakes {
    fn new(lpstat_out: &str, lp_exit: i32, lp_out: &str, lp_err: &str) -> Fakes {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("xc-agent-cups-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let d = dir.display();
        write_script(
            &dir.join("lpstat"),
            &format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{d}/lpstat.args'\n\
                 printf '%s' '{lpstat_out}'\nexit 0\n"
            ),
        );
        write_script(
            &dir.join("lp"),
            &format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{d}/lp.args'\n\
                 cat > '{d}/lp.stdin'\nprintf '%s' '{lp_out}'\nprintf '%s' '{lp_err}' >&2\n\
                 exit {lp_exit}\n"
            ),
        );
        Fakes { dir }
    }

    fn transport(&self, queue: &str) -> CupsTransport {
        CupsTransport::with_commands(queue, self.dir.join("lp"), self.dir.join("lpstat"))
    }

    fn args(&self, name: &str) -> Option<Vec<String>> {
        std::fs::read_to_string(self.dir.join(format!("{name}.args")))
            .ok()
            .map(|t| t.lines().map(str::to_owned).collect())
    }

    fn stdin(&self) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join("lp.stdin")).ok()
    }
}

fn write_script(path: &Path, text: &str) {
    std::fs::write(path, text).expect("write a fake command");
    let mut perms = std::fs::metadata(path).expect("stat").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

const LP_OK: &str = "request id is POS80_-12 (0 file(s))";

#[tokio::test]
async fn an_idle_queue_gets_the_bytes_raw_on_stdin() {
    let fakes = Fakes::new("", 0, LP_OK, "");
    let answer = fakes.transport("POS80_").send(&KICK).await;
    assert_eq!(
        answer,
        Ok(SendReceipt {
            cups_job: Some("POS80_-12".to_owned())
        }),
        "CUPS's job id from `request id is …`"
    );
    assert_eq!(
        fakes.args("lpstat"),
        Some(vec!["-o".to_owned(), "POS80_".to_owned()]),
        "lpstat -o <queue>"
    );
    assert_eq!(
        fakes.args("lp"),
        Some(
            ["-d", "POS80_", "-o", "raw"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        ),
        "lp -d <queue> -o raw"
    );
    assert_eq!(
        fakes.stdin(),
        Some(KICK.to_vec()),
        "the bytes on stdin, exactly"
    );
}

#[tokio::test]
async fn a_pending_job_makes_the_queue_busy_and_nothing_is_sent() {
    let fakes = Fakes::new(
        "POS80_-11              counter          1024   Tue Oct  7 10:00:00 2026\n",
        0,
        LP_OK,
        "",
    );
    let answer = fakes.transport("POS80_").send(&KICK).await;
    assert_eq!(answer, Err(TransportError::QueueBusy));
    assert_eq!(fakes.args("lp"), None, "lp is never run");
}

#[tokio::test]
async fn a_failing_lp_is_a_failure_with_its_message() {
    let fakes = Fakes::new("", 1, "", "lp: The printer or class does not exist.");
    match fakes.transport("POS80_").send(&KICK).await {
        Err(TransportError::Failed(text)) => assert!(
            text.contains("does not exist"),
            "the failure says what lp said: {text}"
        ),
        other => panic!("lp exiting 1 is a failure: {other:?}"),
    }
}

/// No shell is involved: a queue name with shell syntax reaches `lp` as one argument and
/// runs nothing.
#[tokio::test]
async fn the_queue_name_is_one_argument_and_runs_nothing() {
    let fakes = Fakes::new("", 0, LP_OK, "");
    let marker = fakes.dir.join("ran");
    let queue = format!("q;touch {}", marker.display());
    let _ = fakes.transport(&queue).send(&KICK).await;
    assert_eq!(
        fakes.args("lpstat"),
        Some(vec!["-o".to_owned(), queue.clone()])
    );
    assert!(!marker.exists(), "nothing was run through a shell");
}

#[tokio::test]
async fn a_missing_lp_is_a_failure_not_a_panic() {
    let fakes = Fakes::new("", 0, LP_OK, "");
    let transport = CupsTransport::with_commands(
        "POS80_",
        fakes.dir.join("no-such-lp"),
        fakes.dir.join("lpstat"),
    );
    assert!(matches!(
        transport.send(&KICK).await,
        Err(TransportError::Failed(_))
    ));
}
