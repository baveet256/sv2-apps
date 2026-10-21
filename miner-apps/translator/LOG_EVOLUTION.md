# Share-hash log evolution (pre / post each phase)

Living record of what operators see in translator logs **before** and **after** each
audit phase. Fill the “post” section when a phase ships and is live-validated.

Schema: [`SHARE_HASH_AUDIT_SCHEMA.md`](SHARE_HASH_AUDIT_SCHEMA.md)  
Plan: [`SHARE_HASH_AUDIT_PLAN.md`](SHARE_HASH_AUDIT_PLAN.md)

---

## Snapshot index

| Phase | Status | Pre snapshot | Post snapshot |
|-------|--------|--------------|---------------|
| 0 (schema) | done | n/a (docs only) | n/a |
| 1 (structured logs) | done + live | Stock translator (below) | Live Braiins run 2026-10-20 |
| 2 (JSONL) | **done + live** (2026-10-20) | = Phase 1 post (frozen below) | Live Braiins run below |
| 3 (block candidate ops) | **live, no candidate yet** (2026-10-21) | Phase 2 logs (no dedicated file) | Live Braiins run below |
| 4 (chain tip RPC) | not started | — | — |
| 5 (hardening) | not started | — | — |

---

## Pre–Phase 1 (stock translator)

```text
INFO  Received mining.submit from SV1 downstream for channel id: 1
INFO  SubmitSharesExtended: valid share, forwarding ... sequence_number: N ☑️
INFO  Received: SubmitSharesSuccess(...) ✅
```

No `share_hash`. Accept vs forward vs pool ack hard to reconcile by hash.

---

## Post–Phase 1 / Pre–Phase 2 (frozen baseline)

**Captured:** 2026-10-20T23:35:07Z  
**Source:** `tproxy-braiins-phase1.log` (~4257 lines)  
**Setup:** local tProxy → Braiins SV2; miner worker `newtranslator`

### Counts at freeze

| Event | Count |
|-------|------:|
| `share_accepted` | 351 |
| `share_forwarded` | 30 |
| `SubmitSharesSuccess` | 8 |
| `share_not_forwarded` | (debug only; not in info log file) |
| `BLOCK_CANDIDATE` | 0 |

### Sample lines (verbatim)

**Accept:**
```text
INFO share_accepted event="share_accepted" worker="newtranslator" downstream_id=1
     downstream_channel_id=1 job_id=0
     share_hash=0000000000322e4c742618d18cae256b23bcd53c7b31dc84fe747ac8a9a07780
     meets_downstream_target=true
```

**Forward:**
```text
INFO share_forwarded event="share_forwarded" worker="newtranslator"
     downstream_channel_id=1 upstream_channel_id=1 job_id=1 sequence_number=1
     share_hash=000000000000f5cf02386731de55e125659bd02a5aace5f513d85c8d9c2ec8bb
     block_candidate=false
```

**Pool ack (still no hash on this line — Phase 2 correlates via sequence):**
```text
INFO Received: SubmitSharesSuccess(channel_id=1, last_sequence_number=2,
     new_submits_accepted_count=2, new_shares_sum=16384) ✅
```

### What Phase 2 must add (without removing Phase 1 logs)

- Unchanged: stdout/file tracing for `share_accepted` / `share_forwarded`
- **New:** append-only JSONL at `share_audit_path`
  - `share_forwarded` row (with hash + sequence)
  - `share_upstream_result` row when Braiins returns success/error (tie by sequence)
  - `block_candidate` row when applicable

---

## Post–Phase 2 (live validated)

**Captured:** 2026-10-20T23:48Z  
**JSONL path:** `./share-audit-phase2.jsonl`  
**Worker:** `newtranslator1`  
**Stdout:** Phase 1 lines still present (`share_accepted` / `share_forwarded` / `SubmitSharesSuccess` / `SubmitSharesError`)

### JSONL counts at capture

| Event | Count |
|-------|------:|
| `share_forwarded` (JSONL) | 81 |
| `share_upstream_result` success | 71 |
| `share_upstream_result` error | 10 |
| Total JSONL lines | 162 |

### Same story as your terminal (seq 80–81)

**Stdout (Phase 1 — unchanged):**
```text
INFO share_forwarded ... sequence_number=80 share_hash=00000000000272c7... block_candidate=false
WARN SubmitSharesError(... sequence_number=80, error_code=difficulty-too-low) ❌
INFO share_forwarded ... sequence_number=81 share_hash=00000000000397eb...
INFO SubmitSharesSuccess(... last_sequence_number=81, new_submits_accepted_count=1) ✅
```

**JSONL (Phase 2 — new durable trail):**
```json
{"event":"share_forwarded","sequence_number":80,"share_hash":"00000000000272c772c588d1dc0fde7df35ea07f0884547144fca245d71925b3","worker":"newtranslator1",...}
{"event":"share_upstream_result","result":"error","error_code":"difficulty-too-low","sequence_number":80,"share_hash":"00000000000272c772c588d1dc0fde7df35ea07f0884547144fca245d71925b3",...}
{"event":"share_forwarded","sequence_number":81,"share_hash":"00000000000397eb7793ad85e7f08b1d9b4ab1b87dd2ab2d3e8a99601d8ddde5",...}
{"event":"share_upstream_result","result":"success","sequence_number":81,"share_hash":"00000000000397eb7793ad85e7f08b1d9b4ab1b87dd2ab2d3e8a99601d8ddde5",...}
```

**What improved vs Phase 1 alone:** pool ack/nack is now tied to the **same `share_hash`** in a file that survives log scroll. Batched `SubmitSharesSuccess` (e.g. count=4 ending at seq 79) expands into per-sequence success rows with hashes.

- [x] JSONL path used
- [x] Sample forward + success/error with hash correlation
- [x] Phase 1 logs still present
- [x] Counts recorded

---

## Post–Phase 3 (live, no network-difficulty share)

**Captured:** 2026-10-21T18:22Z (session started 18:14:40Z)  
**Worker:** `newtranslator1`  
**Stdout:** Phase 1/2 lines still present. Startup also logged `Block-candidate JSONL enabled path=./block-candidates.jsonl`.  
**Chain tip from upstream:** `SetNewPrevHash ... nbits=0x17020093` (this is the target a `BLOCK_CANDIDATE` must beat).

### Counts at capture

| Event | Count |
|-------|------:|
| `share_accepted` (stdout) | 356 |
| `share_forwarded` (stdout and JSONL this session) | 47 |
| `SubmitSharesSuccess` (stdout) | 13 |
| `share_upstream_result` success (JSONL this session) | 42 |
| `SubmitSharesError` / JSONL error (this session) | 5 |
| `BLOCK_CANDIDATE` | 0 |

Ordinary `share_forwarded` lines do **not** gain `nbits`. That field is only on a real candidate.

### Same story as the terminal (seq 18–22)

**Stdout (unchanged from Phase 2 for ordinary shares):**
```text
INFO share_forwarded event="share_forwarded" worker="newtranslator1"
     downstream_channel_id=1 upstream_channel_id=1 job_id=6 sequence_number=18
     share_hash="00000000000018d658e030802c0310b2cd0862d8a181f9b965135d68045fc91f"
     block_candidate=false
INFO SubmitSharesSuccess(channel_id=1, last_sequence_number=19,
     new_submits_accepted_count=3, new_shares_sum=24576) ✅
INFO share_forwarded ... job_id=8 sequence_number=22
     share_hash="000000000004f8b5159568430577ec160c7cf63f95e46180e9c6f5595ee9915a"
     block_candidate=false
WARN SubmitSharesError(channel_id=1, sequence_number=22, error_code=difficulty-too-low) ❌
```

**Main JSONL (`share-audit-phase2.jsonl`) still correlates the hash.** This session’s error for sequence 12:

```json
{"event":"share_upstream_result","result":"error","error_code":"difficulty-too-low","sequence_number":12,"share_hash":"0000000000015f88201e82703e9938848e7f1856d757f037af88190ad6a8ce7e","worker":"newtranslator1","block_candidate":false}
```

No `block_candidate` event was written. `chain_status` stays absent until a candidate exists, then `unknown` until Phase 4.

### What a real candidate will look like

```text
WARN BLOCK_CANDIDATE meets network target; not a chain-tip confirmation
     event=block_candidate worker=... job_id=... sequence_number=...
     share_hash=0000… nbits=0x17020093 blocks_found=1
     meets_network_target=true chain_status=unknown
```

```json
{"event":"block_candidate","share_hash":"…","nbits":"0x17020093","meets_network_target":true,"chain_status":"unknown","worker":"…","sequence_number":1}
```

### Defect seen in this session

`block-candidates.jsonl` received 5 `share_upstream_result` error rows (`difficulty-too-low`) with `share_hash: null`. Those are ordinary nacks, not candidates. The candidates-only file had no pending forward for them, and the error writer still appended a line. The main audit file kept the real hash. Fixed in `share_audit.rs`: a candidates-only file now skips an error that is not a stored block candidate. The running process keeps the old behavior until the translator is rebuilt and restarted. The 5 rows already in the file stay.

**Checklist when a real candidate appears**
1. Grep `BLOCK_CANDIDATE` in the translator log.
2. Open `block-candidates.jsonl` and copy `share_hash` from an `event=block_candidate` row.
3. Compare that hash to an explorer or `bitcoin-cli getblockheader <hash>` (Phase 4 automates this).
4. Do not treat the log line as “we are on the tip.”

- [x] Phase 3 binary live (`Block-candidate JSONL enabled`)
- [x] Ordinary shares still `block_candidate=false`; main JSONL still has the hash
- [x] `BLOCK_CANDIDATE` count recorded (0)
- [ ] Live `BLOCK_CANDIDATE` line (only if a share meets `nbits`)

---

## Operator notes for phase transitions

1. **Rebuild** translator after code changes (`cargo build -p translator_sv2 --release`).
2. **Restart** tProxy so new config/binary load (Braiins may idle-drop ~15s without a miner — connect miner promptly or use a restart loop).
3. Keep `tproxy-braiins-phase1.log` (or rotate a dated copy) as the frozen Phase 1 baseline; use a **new** JSONL file for Phase 2 (e.g. `share-audit-phase2.jsonl`) so pre/post artifacts don’t mix.
