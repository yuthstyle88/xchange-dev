//! The agent's journal (§5.4c *Restart and offline*): one file of JSON lines, written before
//! the bytes go out and after the outcome is known, so a restart never sends a kick twice.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};

use crate::client::Report;

/// What the journal still owes the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    /// `sending` was written and no outcome: the kick may have gone out.
    Sending { job_id: String, lease_id: String },
    /// An outcome the server has not acknowledged yet.
    Unacknowledged(Report),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JournalEntry {
    Sending {
        job_id: String,
        lease_id: String,
        at: SystemTime,
    },
    Outcome {
        report: Report,
        at: SystemTime,
    },
    Acknowledged {
        job_id: String,
        lease_id: String,
    },
    Rejected {
        job_id: String,
        lease_id: String,
        status: u16,
        reason: String,
    },
}

enum EntryState {
    Sending,
    Unacknowledged { report: Report, at: SystemTime },
}

pub struct Journal {
    pub path: PathBuf,
    file: File,
    fail_writes: bool,
}

impl Journal {
    /// Opens (creating, mode 0600 on Unix) the journal at `path`. A torn last line — a crash
    /// in the middle of a write — is ignored, not an error.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Journal> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = {
            #[cfg(unix)]
            {
                std::fs::OpenOptions::new()
                    .read(true)
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&path)?
            }
            #[cfg(not(unix))]
            {
                std::fs::OpenOptions::new()
                    .read(true)
                    .create(true)
                    .append(true)
                    .open(&path)?
            }
        };

        #[cfg(unix)]
        {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        // Repair torn last line if any: find the last valid JSON entry offset
        repair_torn_last_line(&path, &mut file)?;

        Ok(Journal {
            path,
            file,
            fail_writes: false,
        })
    }

    /// For testing: simulate a failure when writing to the journal.
    pub fn set_fail_writes(&mut self, fail: bool) {
        self.fail_writes = fail;
    }

    fn write_entry(&mut self, entry: &JournalEntry) -> io::Result<()> {
        if self.fail_writes {
            return Err(io::Error::other("simulated journal write failure"));
        }
        let mut line =
            serde_json::to_vec(entry).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()
    }

    /// Written, and flushed to disk, before the bytes are sent.
    pub fn sending(&mut self, job_id: &str, lease_id: &str, at: SystemTime) -> io::Result<()> {
        self.write_entry(&JournalEntry::Sending {
            job_id: job_id.to_owned(),
            lease_id: lease_id.to_owned(),
            at,
        })
    }

    /// Written, and flushed, as soon as the outcome is known — before it is reported.
    pub fn outcome(&mut self, report: &Report, at: SystemTime) -> io::Result<()> {
        self.write_entry(&JournalEntry::Outcome {
            report: report.clone(),
            at,
        })
    }

    /// The server took the report of this lease.
    pub fn acknowledged(&mut self, job_id: &str, lease_id: &str) -> io::Result<()> {
        self.write_entry(&JournalEntry::Acknowledged {
            job_id: job_id.to_owned(),
            lease_id: lease_id.to_owned(),
        })
    }

    /// The server rejected the report of this lease (e.g. 409 conflict, 404).
    /// Recorded in the journal with its status and reason, and no longer owed.
    pub fn rejected(
        &mut self,
        job_id: &str,
        lease_id: &str,
        status: u16,
        reason: &str,
    ) -> io::Result<()> {
        self.write_entry(&JournalEntry::Rejected {
            job_id: job_id.to_owned(),
            lease_id: lease_id.to_owned(),
            status,
            reason: reason.to_owned(),
        })
    }

    /// What is owed, oldest first. An unacknowledged outcome written more than 24 hours
    /// before `now` is no longer owed.
    pub fn pending(&self, now: SystemTime) -> Vec<Pending> {
        let Ok(file) = File::open(&self.path) else {
            return Vec::new();
        };

        let mut reader = BufReader::new(file);
        let mut order = Vec::new();
        let mut states: std::collections::HashMap<(String, String), EntryState> =
            std::collections::HashMap::new();

        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(entry) = serde_json::from_str::<JournalEntry>(trimmed) {
                        match entry {
                            JournalEntry::Sending {
                                job_id,
                                lease_id,
                                at: _,
                            } => {
                                let key = (job_id, lease_id);
                                if !states.contains_key(&key) {
                                    order.push(key.clone());
                                }
                                states.insert(key, EntryState::Sending);
                            }
                            JournalEntry::Outcome { report, at } => {
                                let key = (report.job_id.clone(), report.lease_id.clone());
                                if !states.contains_key(&key) {
                                    order.push(key.clone());
                                }
                                states.insert(key, EntryState::Unacknowledged { report, at });
                            }
                            JournalEntry::Acknowledged { job_id, lease_id }
                            | JournalEntry::Rejected {
                                job_id, lease_id, ..
                            } => {
                                let key = (job_id, lease_id);
                                states.remove(&key);
                                order.retain(|k| k != &key);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }

        const DAY: Duration = Duration::from_secs(24 * 3600);
        let mut result = Vec::new();
        for key in order {
            if let Some(state) = states.get(&key) {
                match state {
                    EntryState::Sending => {
                        result.push(Pending::Sending {
                            job_id: key.0.clone(),
                            lease_id: key.1.clone(),
                        });
                    }
                    EntryState::Unacknowledged { report, at } => {
                        let expired = match now.duration_since(*at) {
                            Ok(elapsed) => elapsed > DAY,
                            Err(_) => false,
                        };
                        if !expired {
                            result.push(Pending::Unacknowledged(report.clone()));
                        }
                    }
                }
            }
        }

        result
    }
}

fn repair_torn_last_line(path: &Path, file: &mut File) -> io::Result<()> {
    let mut data = Vec::new();
    let mut read_file = File::open(path)?;
    read_file.read_to_end(&mut data)?;

    if data.is_empty() {
        return Ok(());
    }

    let mut last_valid_offset = 0;
    let mut current_offset = 0;

    for part in data.split_inclusive(|&b| b == b'\n') {
        let part_len = part.len();
        let trimmed = part.strip_suffix(b"\n").unwrap_or(part);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);

        if trimmed.is_empty() && part.ends_with(b"\n") {
            current_offset += part_len;
            last_valid_offset = current_offset;
            continue;
        }

        if serde_json::from_slice::<JournalEntry>(trimmed).is_ok() {
            current_offset += part_len;
            last_valid_offset = current_offset;
        } else {
            // A torn line in the middle is skipped; continue scanning so
            // valid later lines survive. If this torn line is at the end,
            // last_valid_offset won't include it, and it will be truncated below.
            current_offset += part_len;
        }
    }

    if last_valid_offset < data.len() {
        file.set_len(last_valid_offset as u64)?;
        file.sync_data()?;
        file.seek(SeekFrom::End(0))?;
    } else if !data.ends_with(b"\n") {
        file.write_all(b"\n")?;
        file.sync_data()?;
    }

    Ok(())
}
