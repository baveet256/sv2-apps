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
| 3 (block candidate ops) | not started | — | — |
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

## Operator notes for phase transitions

1. **Rebuild** translator after code changes (`cargo build -p translator_sv2 --release`).
2. **Restart** tProxy so new config/binary load (Braiins may idle-drop ~15s without a miner — connect miner promptly or use a restart loop).
3. Keep `tproxy-braiins-phase1.log` (or rotate a dated copy) as the frozen Phase 1 baseline; use a **new** JSONL file for Phase 2 (e.g. `share-audit-phase2.jsonl`) so pre/post artifacts don’t mix.
