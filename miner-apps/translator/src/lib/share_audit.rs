//! Append-only JSONL share-hash audit trail (Phase 2).
//!
//! When [`TranslatorConfig::share_audit_path`](crate::config::TranslatorConfig) is set,
//! forwarded shares and upstream ack/nack results are written as one JSON object per line.
//! See `SHARE_HASH_AUDIT_SCHEMA.md`.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use tracing::{info, warn};

/// Clonable handle; disabled when constructed with [`ShareAudit::disabled`].
#[derive(Clone, Debug, Default)]
pub struct ShareAudit {
    inner: Option<Arc<ShareAuditInner>>,
}

#[derive(Debug)]
struct ShareAuditInner {
    file: Mutex<File>,
    block_candidates_only: bool,
    /// Pending forwards keyed by (upstream_channel_id, sequence_number).
    pending: Mutex<HashMap<(u32, u32), PendingForward>>,
}

#[derive(Debug, Clone)]
struct PendingForward {
    share_hash: String,
    worker: Option<String>,
    job_id: u32,
    downstream_channel_id: u32,
    block_candidate: bool,
}

/// Fields recorded when a share is forwarded upstream.
pub struct ForwardedShare<'a> {
    pub worker: Option<&'a str>,
    pub downstream_channel_id: u32,
    pub upstream_channel_id: u32,
    pub job_id: u32,
    pub sequence_number: u32,
    pub share_hash: String,
    pub block_candidate: bool,
}

impl ShareAudit {
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn open(path: &Path, block_candidates_only: bool) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        info!(
            path = %path.display(),
            block_candidates_only,
            "Share audit JSONL enabled"
        );
        Ok(Self {
            inner: Some(Arc::new(ShareAuditInner {
                file: Mutex::new(file),
                block_candidates_only,
                pending: Mutex::new(HashMap::new()),
            })),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn record_forwarded(&self, share: ForwardedShare<'_>) {
        let Some(inner) = &self.inner else {
            return;
        };

        let pending = PendingForward {
            share_hash: share.share_hash.clone(),
            worker: share.worker.map(str::to_string),
            job_id: share.job_id,
            downstream_channel_id: share.downstream_channel_id,
            block_candidate: share.block_candidate,
        };

        if share.block_candidate {
            self.write_line(
                inner,
                json!({
                    "event": "block_candidate",
                    "ts": now_rfc3339(),
                    "worker": share.worker,
                    "downstream_channel_id": share.downstream_channel_id,
                    "upstream_channel_id": share.upstream_channel_id,
                    "job_id": share.job_id,
                    "sequence_number": share.sequence_number,
                    "share_hash": share.share_hash,
                    "meets_network_target": true,
                    "block_candidate": true,
                }),
            );
        }

        let should_write_forward =
            share.block_candidate || !inner.block_candidates_only;
        if should_write_forward {
            self.write_line(
                inner,
                json!({
                    "event": "share_forwarded",
                    "ts": now_rfc3339(),
                    "worker": share.worker,
                    "downstream_channel_id": share.downstream_channel_id,
                    "upstream_channel_id": share.upstream_channel_id,
                    "job_id": share.job_id,
                    "sequence_number": share.sequence_number,
                    "share_hash": share.share_hash,
                    "block_candidate": share.block_candidate,
                }),
            );
        }

        // Keep pending when we care about upstream correlation for this share.
        if should_write_forward {
            if let Ok(mut map) = inner.pending.lock() {
                map.insert(
                    (share.upstream_channel_id, share.sequence_number),
                    pending,
                );
            }
        }
    }

    /// Braiins (and others) batch acks: correlate the last `accepted_count` sequences.
    pub fn on_upstream_success(
        &self,
        upstream_channel_id: u32,
        last_sequence_number: u32,
        accepted_count: u32,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        if accepted_count == 0 {
            return;
        }

        let first = last_sequence_number.saturating_sub(accepted_count.saturating_sub(1));
        for sequence_number in first..=last_sequence_number {
            let pending = {
                let Ok(mut map) = inner.pending.lock() else {
                    continue;
                };
                map.remove(&(upstream_channel_id, sequence_number))
            };
            let Some(pending) = pending else {
                continue;
            };
            if inner.block_candidates_only && !pending.block_candidate {
                continue;
            }
            self.write_line(
                inner,
                json!({
                    "event": "share_upstream_result",
                    "ts": now_rfc3339(),
                    "upstream_channel_id": upstream_channel_id,
                    "downstream_channel_id": pending.downstream_channel_id,
                    "sequence_number": sequence_number,
                    "job_id": pending.job_id,
                    "worker": pending.worker,
                    "share_hash": pending.share_hash,
                    "result": "success",
                    "block_candidate": pending.block_candidate,
                }),
            );
        }
    }

    pub fn on_upstream_error(
        &self,
        upstream_channel_id: u32,
        sequence_number: u32,
        error_code: &str,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let pending = {
            let Ok(mut map) = inner.pending.lock() else {
                return;
            };
            map.remove(&(upstream_channel_id, sequence_number))
        };
        let (share_hash, worker, job_id, downstream_channel_id, block_candidate) = match pending {
            Some(p) => {
                if inner.block_candidates_only && !p.block_candidate {
                    return;
                }
                (
                    Some(p.share_hash),
                    p.worker,
                    Some(p.job_id),
                    Some(p.downstream_channel_id),
                    p.block_candidate,
                )
            }
            None => (None, None, None, None, false),
        };

        self.write_line(
            inner,
            json!({
                "event": "share_upstream_result",
                "ts": now_rfc3339(),
                "upstream_channel_id": upstream_channel_id,
                "downstream_channel_id": downstream_channel_id,
                "sequence_number": sequence_number,
                "job_id": job_id,
                "worker": worker,
                "share_hash": share_hash,
                "result": "error",
                "error_code": error_code,
                "block_candidate": block_candidate,
            }),
        );
    }

    fn write_line(&self, inner: &ShareAuditInner, value: Value) {
        let Ok(mut file) = inner.file.lock() else {
            warn!("share audit: failed to lock JSONL file");
            return;
        };
        if let Err(e) = writeln!(file, "{value}") {
            warn!(error = %e, "share audit: failed to write JSONL line");
            return;
        }
        if let Err(e) = file.flush() {
            warn!(error = %e, "share audit: failed to flush JSONL file");
        }
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn writes_forward_and_success_with_hash_correlation() {
        let dir = std::env::temp_dir().join(format!(
            "tproxy-share-audit-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");

        let audit = ShareAudit::open(&path, false).unwrap();
        audit.record_forwarded(ForwardedShare {
            worker: Some("rig1"),
            downstream_channel_id: 1,
            upstream_channel_id: 7,
            job_id: 42,
            sequence_number: 10,
            share_hash: "aa".repeat(32),
            block_candidate: false,
        });
        audit.on_upstream_success(7, 10, 1);

        let file = File::open(&path).unwrap();
        let lines: Vec<String> = BufReader::new(file)
            .lines()
            .map(|l| l.unwrap())
            .collect();
        assert_eq!(lines.len(), 2);

        let forward: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(forward["event"], "share_forwarded");
        assert_eq!(forward["sequence_number"], 10);
        assert_eq!(forward["share_hash"], "aa".repeat(32));

        let result: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(result["event"], "share_upstream_result");
        assert_eq!(result["result"], "success");
        assert_eq!(result["share_hash"], "aa".repeat(32));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_candidates_only_skips_ordinary_forwards() {
        let dir = std::env::temp_dir().join(format!(
            "tproxy-share-audit-bco-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");

        let audit = ShareAudit::open(&path, true).unwrap();
        audit.record_forwarded(ForwardedShare {
            worker: Some("rig1"),
            downstream_channel_id: 1,
            upstream_channel_id: 7,
            job_id: 1,
            sequence_number: 1,
            share_hash: "bb".repeat(32),
            block_candidate: false,
        });
        audit.record_forwarded(ForwardedShare {
            worker: Some("rig1"),
            downstream_channel_id: 1,
            upstream_channel_id: 7,
            job_id: 1,
            sequence_number: 2,
            share_hash: "cc".repeat(32),
            block_candidate: true,
        });

        let file = File::open(&path).unwrap();
        let lines: Vec<String> = BufReader::new(file)
            .lines()
            .map(|l| l.unwrap())
            .collect();
        // block_candidate event + share_forwarded for the candidate only
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l.contains("block_candidate")));
        assert!(lines.iter().any(|l| l.contains("share_forwarded")));
        assert!(!lines.iter().any(|l| l.contains(&"bb".repeat(32))));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
