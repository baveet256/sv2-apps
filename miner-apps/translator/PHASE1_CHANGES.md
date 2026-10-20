# Phase 1 changes — share hash audit (what changed & why it’s clearer)

Branch: `2026-10-15-tproxy-share-hash-audit`  
Status: **Phase 1 complete and validated live** (Braiins SV2 + local miner worker `newtranslator`, 2026-10-20).

Companion docs:

- Plan: [`SHARE_HASH_AUDIT_PLAN.md`](SHARE_HASH_AUDIT_PLAN.md)
- Schema contract: [`SHARE_HASH_AUDIT_SCHEMA.md`](SHARE_HASH_AUDIT_SCHEMA.md)

---

## Problem before this work

Operators only saw protocol crumbs like `mining.submit` / “forwarding share” / `SubmitSharesSuccess`.

- No Bitcoin **header hash** in normal logs
- No way to answer “what hash did we send upstream?” without reconstructing from notify+submit
- `BlockFound` (network difficulty) was counted internally but not treated as a loud event
- Accept vs “actually forwarded to pool” was easy to confuse under vardiff

---

## What we changed (code)

| Change | Where | Why it’s better |
|--------|--------|-----------------|
| Structured `share_accepted` log with `share_hash` | `src/lib/sv1/sv1_server/downstream_message_handler.rs` | Every locally valid share gets an explorer-style hash immediately |
| Carry hash + worker on the SV1→CM path | `Sv1ShareMetadata`, `SubmitShareWithChannelId.share_hash` in `src/lib/utils.rs` | Same hash can be correlated through accept → forward / drop |
| Match `Valid(hash)` vs `BlockFound(hash)` | `src/lib/sv2/channel_manager/mod.rs` | Forward path logs the hash; network-difficulty shares get `BLOCK_CANDIDATE` (warn) |
| Structured `share_forwarded` | same | “Hashes sent upstream” are first-class, with `sequence_number` + both channel ids |
| Structured `share_not_forwarded` (debug) | same | Explains vardiff drops that never reach the pool |
| Phase 0 schema + plan | `SHARE_HASH_AUDIT_SCHEMA.md`, `SHARE_HASH_AUDIT_PLAN.md` | Locked vocabulary so later phases don’t redefine meanings |

Hash encoding: **bitcoind / explorer display hex** (`BlockHash` / share hash `Display`).

---

## Before vs after (real shape)

### Before (stock)

```text
INFO  Received mining.submit from SV1 downstream for channel id: 1
INFO  SubmitSharesExtended: valid share, forwarding ... channel_id: 1, sequence_number: 3 ☑️
INFO  Received: SubmitSharesSuccess(...) ✅
```

You know *that* a share moved. You do **not** know *which header hash*.

### After Phase 1 (your live run)

Local accept (many of these — miner difficulty):

```text
INFO share_accepted event="share_accepted" worker="newtranslator"
     downstream_id=1 downstream_channel_id=1 job_id=0
     share_hash=0000000000322e4c742618d18cae256b23bcd53c7b31dc84fe747ac8a9a07780
     meets_downstream_target=true
```

Actually sent upstream (harder; fewer):

```text
INFO share_forwarded event="share_forwarded" worker="newtranslator"
     downstream_channel_id=1 upstream_channel_id=1 job_id=1 sequence_number=1
     share_hash=000000000000f5cf02386731de55e125659bd02a5aace5f513d85c8d9c2ec8bb
     block_candidate=false
```

Pool ack (still no per-hash field from Braiins — Phase 2 will correlate by sequence):

```text
INFO Received: SubmitSharesSuccess(channel_id=1, last_sequence_number=2, ...) ✅
```

### How to read your snippet

The terminal slice you pasted is **mostly `share_accepted`**. That alone is already better than pre-change: each line has a concrete `share_hash`.

Not every accept becomes a forward — under vardiff, local target is often easier than the upstream channel target. Grep the full log for:

```bash
rg 'share_accepted|share_forwarded|SubmitSharesSuccess' tproxy-braiins-phase1.log
```

Live session counts from `tproxy-braiins-phase1.log` (same run):

| Event | Count |
|-------|------:|
| `share_accepted` | 310 |
| `share_forwarded` | 14 |
| Braiins `SubmitSharesSuccess` | 4 |
| `share_not_forwarded` | 0 in file (emitted at **debug**; most accepts are vardiff-filtered before forward — enable debug to see drops) |

That gap (310 accepts → 14 forwards) is expected under vardiff and is exactly why logging both layers matters.

---

## Clarity gains checklist

| Question | Before | After Phase 1 |
|----------|--------|----------------|
| What header hash did this miner produce? | Reconstruct manually | `share_accepted.share_hash` |
| Did it go to the pool? | Ambiguous | Look for matching `share_forwarded` |
| Who submitted it? | Hard | `worker=` |
| Upstream wire identity of the share | Sequence only, no hash | `sequence_number` + `share_hash` on forward |
| Possible Bitcoin block? | Silent counter bump | `BLOCK_CANDIDATE` warn (none expected on normal shares) |
| Durable file across restarts | No | **Phase 2** |
| On Bitcoin tip / orphan? | No | **Phase 4** |

---

## Identity reminder (Braiins test)

| Setting | Value |
|---------|--------|
| tProxy `user_identity` | Braiins account only, e.g. `baveetstudy` |
| Miner SV1 username | Worker label, e.g. `newtranslator` |
| Upstream | Braiins **SV2** (`authority_pubkey` + Noise), not classic SV1 stratum URL as upstream |

Config used: `config-examples/mainnet/tproxy-config-braiins-local-test.toml`

---

## What’s next

| Phase | Deliverable |
|-------|-------------|
| **2 (next)** | Append-only **JSONL** at `share_audit_path` for every forward (+ upstream success/error by sequence) so records survive log scroll/rotation |
| **3** | First-class block-candidate ops treatment (always audit candidates) |
| **4** | bitcoind RPC: tip / orphan / confirmations for candidate hashes |
| **5** | Config examples, README, production hardening |

Suggested next implementation step: **Phase 2 JSONL**.

---

## How to verify anytime

```bash
# From miner-apps/translator while tProxy is running:
rg 'event="share_accepted"' tproxy-braiins-phase1.log | tail
rg 'event="share_forwarded"' tproxy-braiins-phase1.log | tail
rg 'SubmitSharesSuccess' tproxy-braiins-phase1.log | tail
rg 'BLOCK_CANDIDATE' tproxy-braiins-phase1.log   # expect empty in normal mining
```
