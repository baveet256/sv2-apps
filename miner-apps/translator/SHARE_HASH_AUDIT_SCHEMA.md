# Share Hash Audit Schema (Phase 0 — frozen)

This document is the **vocabulary and contract** for share-hash auditing in Translator Proxy.
Later phases implement against this file; they should not redefine meanings here without an
explicit schema revision.

Status: **Phase 0–2 complete** (schema + structured logs + JSONL). Live JSONL post-check pending restart.  
Companion plan: [`SHARE_HASH_AUDIT_PLAN.md`](SHARE_HASH_AUDIT_PLAN.md).  
Log timeline: [`LOG_EVOLUTION.md`](LOG_EVOLUTION.md).

---

## 1. What a `share_hash` is

| Item | Definition |
|------|------------|
| Object | Bitcoin **block header hash** for the share |
| Computation | Double-SHA256 of the 80-byte block header rebuilt from the SV1 job + submit (same as `Header::block_hash()` today in `validate_sv1_share`) |
| Not | A stratum job id, extranonce, merkle root alone, or “confirmed on-chain block” |

### Hex encoding (locked)

| Rule | Choice |
|------|--------|
| Display | **bitcoind / block-explorer order** — the 64-char hex string equal to what `bitcoin-cli getblockhash` / explorers show for that header hash |
| Case | Lowercase hex |
| Length | Exactly 64 characters when present |
| Implementation hint (Phase 1+) | Prefer `BlockHash`’s Display / RPC-conventional string; do **not** invent a second endianness. If comparing to current `debug!` target hex in `utils.rs`, verify against a known mainnet/signet header before shipping. |

**Caution:** Wrong endianness makes hashes unverifiable against explorers and Phase 4 RPC.

---

## 2. Vocabulary (do not blur these)

| Term | Meaning |
|------|---------|
| **share_accepted** | Local SV1 validation passed (meets downstream/miner target, not duplicate). Miner may already have received `true`. |
| **share_forwarded** | Share passed upstream-channel validation and was sent to the SV2 upstream as `SubmitSharesExtended`. |
| **share_not_forwarded** | Accepted locally (or validated) but **not** sent upstream (e.g. does not meet upstream channel target under vardiff). |
| **block_candidate** | Share hash meets Bitcoin **network** difficulty (`ShareValidationResult::BlockFound`). Rare. Not proof of chain tip. |
| **share_upstream_result** | Upstream `SubmitSharesSuccess` or `SubmitSharesError` correlated to a prior forward. |
| **chain_status** | Result of asking Bitcoin (bitcoind RPC) whether a hash is known / in best chain (Phase 4). |

Hard rules:

- Do **not** call every submitted or accepted share a “block.”
- `block_candidate` ≠ in best chain.
- `not_found` from RPC ≠ orphan (may still be propagation delay).

---

## 3. Event names (locked)

Use these exact `event` string values in structured logs and (later) JSONL:

| `event` | Phase | When |
|---------|-------|------|
| `share_accepted` | 1 | After local `MeetsTarget` + dedup insert |
| `share_not_forwarded` | 1 | Dropped before upstream send (e.g. upstream target) |
| `share_forwarded` | 1 | After upstream validation, message queued/sent upstream |
| `block_candidate` | 1 / 3 | Network difficulty met (`BlockFound`) |
| `share_upstream_result` | 2 | Upstream success or error for a sequence |
| `chain_status` | 4 | RPC tip/orphan (or not-found) update |

---

## 4. Field dictionary

### 4.1 Common fields

| Field | Type | Required on | Notes |
|-------|------|-------------|-------|
| `event` | string | all | One of the locked event names |
| `ts` | string (RFC3339 / ISO-8601 UTC) | JSONL always; logs when structured | Wall clock at emit time |
| `share_hash` | string (64 hex) | accepted, forwarded, not_forwarded, block_candidate, chain_status | See §1 |
| `worker` | string | preferred whenever known | SV1 worker / authorize name |
| `downstream_id` | u32 / number | preferred on SV1-side events | Local downstream id |
| `downstream_channel_id` | u32 | preferred | Channel id as known on the downstream / pre-remap side |
| `upstream_channel_id` | u32 | preferred on forward+ | Channel id on the wire to upstream (after aggregated remap) |
| `job_id` | string or number | preferred | Job the share refers to (SV1 job id string and/or SV2 u32 as available) |
| `sequence_number` | u32 | **required** on `share_forwarded`, `share_upstream_result` | **Upstream** sequence actually sent (after aggregated rewrite) |
| `meets_downstream_target` | bool | accepted | Should be `true` if event is `share_accepted` |
| `block_candidate` | bool | forwarded, block_candidate | `true` iff network target met |
| `meets_network_target` | bool | optional alias | Prefer `block_candidate` as the canonical flag |
| `reason` | string | `share_not_forwarded`, errors | Stable snake_case reason codes when possible |
| `network` | string | optional until known | `mainnet` \| `testnet` \| `testnet4` \| `signet` \| `regtest` |
| `result` | string | `share_upstream_result` | `success` \| `error` |
| `error_code` | string | upstream error | From `SubmitSharesError` when present |
| `chain_status` | string | `chain_status` event | See §4.2 |
| `confirmations` | number | `chain_status` when in best chain | From bitcoind |
| `checked_at` | string (RFC3339) | `chain_status` | When RPC was queried |

### 4.2 `chain_status` values (Phase 4; reserved now)

| Value | Meaning |
|-------|---------|
| `unknown` | Not checked yet |
| `not_found` | Node does not know this block hash (yet) |
| `in_best_chain` | Header/block is in the best chain |
| `orphaned_or_stale` | Known to the node but not in the best chain — use carefully; do not set too early |

### 4.3 Optional forensic fields (not required for v1)

Enabled only behind a future flag if needed: `prev_hash`, `merkle_root`, `version`, `ntime`, `bits`, `nonce`.  
Default audit trail is **hash + identity/correlation ids**, not full header dumps.

---

## 5. Correlation rules

| Goal | Key |
|------|-----|
| Tie local accept → forward | Same `share_hash` (+ `worker` / `downstream_id` when present) |
| Tie forward → upstream ack/nack | `(upstream_channel_id, sequence_number)` — must be the sequence **after** aggregated rewrite |
| Tie candidate → chain check | `share_hash` |

In aggregated mode, always prefer logging **both** `downstream_channel_id` and `upstream_channel_id` on forward events.

---

## 6. Config knobs (locked names; wired in later phases)

TOML / env follow existing Translator conventions (`TPROXY__*` with `__` nesting).

| Key | Type | Default | Phase wired | Meaning |
|-----|------|---------|-------------|---------|
| `share_audit_path` | optional path | unset / empty | 2 | Append-only JSONL file. Unset = logs only (Phase 1 behavior). |
| `share_audit_block_candidates_only` | bool | `false` | 2 | If `true`, JSONL records only `block_candidate` (+ their follow-up events). Logs for accept/forward still follow Phase 1 policy. |

### Reserved for Phase 4 (names provisional, not implemented in Phase 0–3)

| Key | Intent |
|-----|--------|
| `share_audit_bitcoin_rpc_url` | bitcoind RPC endpoint |
| `share_audit_bitcoin_rpc_auth` / cookie path | Credentials (never commit) |
| `share_audit_chain_poll_*` | Backoff / TTL / confirmations stop |

Env examples (Phase 2+):

```bash
TPROXY__SHARE_AUDIT_PATH=/var/log/tproxy/share-audit.jsonl
TPROXY__SHARE_AUDIT_BLOCK_CANDIDATES_ONLY=false
```

---

## 7. Logging policy (Phase 1 contract)

Agreed for farm-scale use:

| Event | Level | Notes |
|-------|-------|-------|
| `share_accepted` | info (structured) | Include `share_hash` |
| `share_forwarded` | info (structured) | Include `share_hash`, `sequence_number`, `block_candidate` |
| `share_not_forwarded` | debug | Include `share_hash` + `reason` |
| `block_candidate` | warn | Distinct marker string `BLOCK_CANDIDATE` allowed in message; fields still structured |

Do not log full coinbase blobs or private job templates.

---

## 8. JSONL policy (Phase 2 contract; preview)

- One JSON object per line; **append-only** (no in-place updates).
- Prefer **two events** for ack correlation: `share_forwarded` then `share_upstream_result`.
- `block_candidate` rows are always eligible for JSONL even if quieter share audit is narrowed later.
- Rotation / retention: **ops** (e.g. logrotate), not in-process in v1.
- Flush: each write (durability over throughput) unless later measured otherwise.

Example lines:

```json
{"event":"share_forwarded","ts":"2026-10-15T17:01:02Z","worker":"farm1.rig07","downstream_channel_id":12,"upstream_channel_id":7,"job_id":"42","sequence_number":1088,"share_hash":"00000000000000000002a1b2c3d4e5f678901234567890abcdef1234567890ab","block_candidate":false}
{"event":"share_upstream_result","ts":"2026-10-15T17:01:02.050Z","upstream_channel_id":7,"sequence_number":1088,"result":"success","share_hash":"00000000000000000002a1b2c3d4e5f678901234567890abcdef1234567890ab"}
```

---

## 9. Out of scope for this schema

- Monitoring HTTP history API
- Webhooks / Slack
- Pushing candidate blocks to the P2P network from tProxy
- AZCoin-specific chain rules (Bitcoin semantics only)

---

## 10. Phase 0 exit checklist

- [x] Field dictionary frozen (§4)
- [x] Hash hex = bitcoind/explorer display order (§1)
- [x] Config key `share_audit_path` locked (§6)
- [x] Event names locked (§3)
- [x] Accept + forward both in Phase 1 logging policy (§7)
- [x] Tip/orphan explicitly deferred to Phase 4 vocabulary only

**Phases 0–2 done in code.** Next live step: rebuild/restart with `share_audit_path`, then fill Post–Phase 2 in `LOG_EVOLUTION.md`. Phase 3 = block-candidate ops prominence.
