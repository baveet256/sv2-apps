# Share Hash Audit Plan (SV1 → SV2 Translator)

Goal: record Bitcoin share/header hashes that this translator forwards upstream, surface potential block finds (`BlockFound` / network difficulty), and later check whether those hashes appear in Bitcoin’s best chain or are orphaned/stale.

Context: the translator already computes each share’s header hash for validation and dedup, and the SV2 channel layer can return `BlockFound(hash)`, but application logs and monitoring do not expose a usable per-share hash trail. Chain tip vs orphan cannot be known from the translator alone.

Related upstream tree: [sv2-apps/miner-apps/translator](https://github.com/stratum-mining/sv2-apps/tree/main/miner-apps/translator).

---

## Locked defaults

These are the defaults for implementation unless overridden before Phase 1:

| Decision | Default | Status |
|----------|---------|--------|
| What to record | **Tiered:** quieter trail for every share forwarded upstream + loud events for Bitcoin network-difficulty / `BlockFound` | locked |
| Persistence | **Structured logs + append-only JSONL** (no DB, no monitoring history API in v1) | locked |
| Chain semantics | **Bitcoin-only** (header hash = double-SHA256 of the reconstructed block header) | locked |
| Tip / orphan | **Later phase** via bitcoind RPC (not explorer scraping first) | locked |
| Hash hex display | **bitcoind / explorer display order** (match `getblockhash` / block explorers) | **confirmed Phase 0** |
| JSONL path | Config key `share_audit_path`; empty / unset = logs only | **confirmed Phase 0** |
| Phase 1 log points | **Both accepted and forwarded** (with hash on each) | **confirmed Phase 1** |
| Phase 1 volume | Accept + forward at info-friendly farm scale; `BlockFound` louder (warn) | **confirmed Phase 1** |
| Block push | **Record only** — pool/JDC owns network submission | locked |
| Alerts beyond logs | Logs + JSONL only in early phases (no Slack/webhook unless added later) | locked |

---

## Examples (what you would see)

These are illustrative Phase 1 log lines — not final wording. Same fake hash used across a successful path so you can follow one share.

### Pipeline picture

```text
Miner submits share
        |
        v
 [1] ACCEPTED  = tProxy rebuilt the header, hash meets THIS miner's
                 difficulty, not a duplicate. Miner already got `true`.
                 Hash is known here.
        |
        v
 [2] Maybe DROPPED = meets miner difficulty but NOT the harder upstream
                     channel target (common with vardiff). Logged quieter.
                     Never sent to the pool.
        |
        v
 [3] FORWARDED = passed upstream-channel validation; SubmitSharesExtended
                 sent to SV2 pool/JDC. This is "hashes sent upstream."
        |
        +-- normal Valid share
        +-- BLOCK_CANDIDATE = same path, but hash also met Bitcoin
                              network difficulty (extremely rare)
        |
        v
 [4] Upstream Success / Error  (Phase 2 correlates by sequence_number)
```

### Example A — Normal share (accepted + forwarded)

What happens: good share for the miner, also hard enough for the upstream channel.

```text
INFO share_accepted  worker=farm1.rig07 downstream_id=3 channel_id=12
     job_id=42 share_hash=00000000000000000002a1b2c3d4e5f678901234567890abcdef1234567890ab
     meets_downstream_target=true

INFO share_forwarded worker=farm1.rig07 downstream_channel_id=12 upstream_channel_id=7
     job_id=42 sequence_number=1088
     share_hash=00000000000000000002a1b2c3d4e5f678901234567890abcdef1234567890ab
     block_candidate=false
```

**How to read it:** same `share_hash` twice = one share’s life. First line = we accepted it locally. Second = we sent it upstream. At farm scale you expect many of these pairs per minute.

### Example B — Accepted locally, not forwarded (vardiff drop)

What happens: miner difficulty is easier than the aggregated upstream target. tProxy still ACKs the miner (so hashrate accounting works) but does **not** send that share to the pool.

```text
INFO  share_accepted  worker=farm1.rig07 ... share_hash=0000...90ab meets_downstream_target=true

DEBUG share_not_forwarded reason=does_not_meet_upstream_target
      worker=farm1.rig07 channel_id=12 share_hash=0000...90ab
```

**How to read it:** you still have the hash (accepted), but it is **not** in the “sent upstream” audit. This is why logging **both** matters — otherwise you cannot tell “miner found work” from “pool received work.”

### Example C — Block candidate (network difficulty)

What happens: same as A, but the header hash also beats Bitcoin’s current network target. Still not proof it is on the tip — only that it is a real block candidate from tProxy’s view.

```text
INFO  share_accepted  ... share_hash=00000000000000000001deadbeef... 

WARN  BLOCK_CANDIDATE worker=farm1.rig07 job_id=42 sequence_number=1090
      share_hash=00000000000000000001deadbeefcafe00112233445566778899aabbccddeeff
      meets_network_target=true

INFO  share_forwarded ... share_hash=00000000000000000001deadbeef... block_candidate=true
```

**How to read it:** grep `BLOCK_CANDIDATE` in ops. Later Phase 4 asks bitcoind: is this hash in the best chain or orphaned/stale?

### Example D — Phase 2 JSONL (preview; not Phase 1)

Append-only file from `share_audit_path`, one JSON object per line:

```json
{"event":"share_forwarded","ts":"2026-10-15T17:01:02Z","worker":"farm1.rig07","share_hash":"00000000000000000002a1b2...","sequence_number":1088,"block_candidate":false}
{"event":"share_upstream_result","ts":"2026-10-15T17:01:02.050Z","sequence_number":1088,"result":"success"}
{"event":"block_candidate","ts":"2026-10-15T17:08:11Z","worker":"farm1.rig07","share_hash":"00000000000000000001dead...","sequence_number":1090}
{"event":"chain_status","ts":"2026-10-15T17:20:00Z","share_hash":"00000000000000000001dead...","status":"in_best_chain","confirmations":3}
```

**How to read it:** logs are for live eyes; JSONL is the durable trail you can `jq` / ship to storage after restarts.

### Volume intuition (farm scale)

Assume `shares_per_minute = 6` and 50 miners, most shares also meet upstream target:

| Event | Rough rate |
|-------|------------|
| `share_accepted` | ~300 / min |
| `share_forwarded` | ~same order (minus vardiff drops) |
| `BLOCK_CANDIDATE` | ~never in practice; when it fires, treat as PagerDuty-level rare |

Both accepted + forwarded at info is intentional: you can reconcile miner ACK vs upstream send. If logs get too chatty later, dial **accepted** down to debug without losing the upstream hash trail.

---

## Local testing (miner → translator → pool)

Yes — the practical way to validate this work is:

```text
ASIC / miner (SV1)  -->  localhost:tProxy.downstream_port  -->  SV2 upstream (pool / JDC)
                              |
                              +--> stdout / log file  (+ JSONL from Phase 2)
```

1. Build/run translator from this branch with a config whose `[[upstreams]]` points at your SV2 pool.
2. Point **one** miner’s stratum URL at the translator’s `downstream_address`:`downstream_port` (not directly at the pool).
3. Watch logs for `share_accepted` / `share_forwarded` (and later JSONL / `BLOCK_CANDIDATE` / chain status).

Keep NTP synced on the translator host (existing cert warning still applies).

### AZCoin pool vs Bitcoin pool — what to use when

The translator always builds a **Bitcoin-shaped header** and hashes it with Bitcoin double-SHA256 (`Header::block_hash()`). That is fine for *seeing* accept/forward log shape on any SHA256d-style SV2 upstream you already used with tProxy (including AZCoin).

| Phase | Can AZCoin upstream validate the *logging*? | Prefer Bitcoin? | Why |
|-------|---------------------------------------------|-----------------|-----|
| **1** (structured logs) | **Yes — good enough** | Optional | You mainly need real `mining.submit` traffic and readable `share_hash` lines |
| **2** (JSONL) | **Yes** | Optional | Same; durable file of forwarded hashes |
| **3** (`BLOCK_CANDIDATE`) | **Only loosely** | Prefer **Bitcoin signet / testnet4** (or local SV2 + bitcoind stack) | “Network target” comes from job `nbits` / chain tip. AZCoin difficulty ≠ Bitcoin mainnet meaning; a candidate on AZCoin is not a Bitcoin block |
| **4** (tip / orphan via RPC) | **No** | **Bitcoin only** | Needs bitcoind (`getblockheader` / best chain). AZCoin has no Bitcoin tip |

**Recommendation**

- **Phases 1–2 (now):** point one miner at local tProxy with your **AZCoin SV2 upstream** if that is what you already run. Fastest path to prove accept/forward/hash lines and (soon) JSONL.
- **Phases 3–4:** switch validation to a **Bitcoin** path — prefer **signet or testnet4** + matching example configs under `config-examples/`, not mainnet, unless you intentionally want mainnet ops rehearsal. Use mainnet only when you care about production wiring; do not expect to “see” `BLOCK_CANDIDATE` on purpose (it is extremely rare).
- Do **not** use AZCoin results to claim a Bitcoin chain-tip / orphan outcome.

If AZCoin’s PoW/header rules ever diverge from Bitcoin’s 80-byte header + SHA256d, Phase 1 hashes remain “what tProxy computed,” not necessarily what AZCoin explorers show — still useful for audit of *this* proxy.

---

## Log evolution by phase (keep this as the record)

What operators see **before** this work vs **after each phase**. Fake hashes abbreviated as `0000…abcd`.

### Before (stock translator)

```text
INFO  Received mining.submit from SV1 downstream for channel id: 12
INFO  SubmitSharesExtended: valid share, forwarding it to upstream | channel_id: 7, sequence_number: 1088 ☑️
INFO  Received: SubmitSharesSuccess { ... } ✅
```

- No `share_hash`
- No distinction for network-difficulty / `BlockFound`
- Monitoring: aggregate counters only (`shares_submitted`, `blocks_found`)

### After Phase 0 (docs only)

No runtime log change. Contract frozen in `SHARE_HASH_AUDIT_SCHEMA.md`.

### After Phase 1 (current) — structured logs + hash

```text
INFO  Received mining.submit from SV1 downstream for channel id: 12

INFO  event=share_accepted worker=farm1.rig07 downstream_id=3 downstream_channel_id=12
      job_id=42 share_hash=0000…abcd meets_downstream_target=true
      share_accepted

INFO  event=share_forwarded worker=farm1.rig07 downstream_channel_id=12 upstream_channel_id=7
      job_id=42 sequence_number=1088 share_hash=0000…abcd block_candidate=false
      share_forwarded

# Rare — network difficulty met:
WARN  event=block_candidate ... share_hash=0000…dead ... BLOCK_CANDIDATE

# Vardiff drop (needs debug level):
DEBUG event=share_not_forwarded ... share_hash=0000…abcd reason=does_not_meet_upstream_target
```

**What changed:** every accepted/forwarded share carries explorer-style `share_hash`. Grep `share_forwarded` or `BLOCK_CANDIDATE`. Still **no** durable file; still **no** upstream ack tied to hash in a file; tip/orphan unknown.

**How to check locally:** run tProxy → point one miner → `grep share_forwarded` / `grep share_accepted` on the process log.

### After Phase 2 — same logs + JSONL file

Stdout/stderr: same as Phase 1.

**New:** append-only file at `share_audit_path`:

```json
{"event":"share_forwarded","ts":"…","worker":"farm1.rig07","share_hash":"0000…abcd","sequence_number":1088,"block_candidate":false,…}
{"event":"share_upstream_result","ts":"…","sequence_number":1088,"result":"success","share_hash":"0000…abcd",…}
```

**What changed:** records survive log rotation / restart. Upstream success/error becomes a second JSON line keyed by `(upstream_channel_id, sequence_number)`.

### After Phase 3 — loud candidates always audited

Logs: Phase 1 style, with `BLOCK_CANDIDATE` treated as the ops-critical line.

JSONL: candidate rows always written even if `share_audit_block_candidates_only = true` (ordinary forwards optional).

**What changed:** ops can grep one marker; durable trail cannot “forget” a candidate because quieter share audit was disabled.

### After Phase 4 — chain status (Bitcoin RPC)

Logs (illustrative):

```text
INFO  event=chain_status share_hash=0000…dead status=in_best_chain confirmations=3
# or
INFO  event=chain_status share_hash=0000…dead status=not_found
# or later
INFO  event=chain_status share_hash=0000…dead status=orphaned_or_stale
```

JSONL adds matching `chain_status` events.

**What changed:** answers “did this hash land in Bitcoin’s best chain?” — **Bitcoin bitcoind only**, not AZCoin.

### After Phase 5 — ops handoff

No new event types required. Config examples + README explain the above; default remains safe if audit path unset (Phase 1 logs only).

---

## Important distinctions

| Term | Meaning |
|------|---------|
| **Submitted / share hash** | Bitcoin block header hash of the share |
| **Block candidate (`BlockFound`)** | Hash meets Bitcoin **network** target at validation time |
| **In best chain / tip ancestry** | bitcoind (or equivalent) knows the block and it is in the best chain |
| **Orphan / stale** | Known to the node but not in the best chain (word carefully; timing matters) |

Do **not** call every submitted share a “block.”  
`BlockFound` ≠ “on chain.”  
“Not found yet” ≠ orphan (propagation delay).

---

## Architecture (target)

```text
SV1 miner
  mining.submit
       |
       v
 validate_sv1_share  -->  header_hash (already computed today)
       |
       v
 accept + dedup  -->  structured accept log (hash)
       |
       v
 SV1 -> SV2 SubmitSharesExtended
       |
       v
 ChannelManager validate_share
    |-- Valid(hash)       --> forward upstream + audit event
    |-- BlockFound(hash)  --> loud BLOCK_CANDIDATE + always audit
       |
       v
 Upstream SubmitSharesSuccess / Error  --> correlate by channel_id + sequence_number
       |
       v
 JSONL audit trail  -->  (Phase 4) bitcoind RPC chain status
```

### Key code touchpoints (today)

| Piece | Path |
|-------|------|
| Local header validation / hash | `src/lib/utils.rs` — `validate_sv1_share` |
| SV1 submit accept + dedup | `src/lib/sv1/sv1_server/downstream_message_handler.rs` — `handle_submit` |
| SV1 → SV2 convert + send | `src/lib/sv1/sv1_server/mod.rs` — `handle_submit_shares` |
| Upstream validate + forward | `src/lib/sv2/channel_manager/mod.rs` — `SubmitSharesExtended` arm |
| Upstream ack / nack | `src/lib/sv2/channel_manager/mining_message_handler.rs` |
| Monitoring aggregates only | `src/lib/monitoring.rs` (`blocks_found` counter, no per-share hashes) |

---

## Proposed audit record fields

Minimum fields for one logical share:

| Field | Notes |
|-------|--------|
| `timestamp` | ISO-8601 / RFC3339 |
| `event` | e.g. `share_accepted`, `share_forwarded`, `share_upstream_result`, `block_candidate`, `chain_status` |
| `worker` | SV1 worker / user identity |
| `downstream_id` | Local downstream id |
| `channel_id` | Prefer logging both downstream and upstream ids in aggregated mode |
| `job_id` | Job the share refers to |
| `sequence_number` | **Upstream** sequence actually sent (after aggregated rewrite) |
| `share_hash` | Header hash hex (display order) |
| `meets_downstream_target` | bool |
| `meets_network_target` / `block_candidate` | bool |
| `upstream_forwarded` | bool |
| `upstream_ack` / `upstream_error` | From Phase 2 correlation |
| `network` | mainnet / testnet / signet when known |
| `confirmations` / `chain_status` | Phase 4 |

Optional forensic mode (larger): `prev_hash`, `merkle_root`, `ntime`, `bits`, `nonce`, `version`.

---

## Phase 0 — Align semantics (no share-path code)

**Status: COMPLETE**

**Goal:** Agree vocabulary so logs are unambiguous later.

**Deliverable:** Frozen contract in [`SHARE_HASH_AUDIT_SCHEMA.md`](SHARE_HASH_AUDIT_SCHEMA.md)
(events, fields, hash hex rules, config knob names, Phase 1 log levels).

**Resolved**
- Hex byte order: **bitcoind / explorer display**
- JSONL path: config key **`share_audit_path`**
- Phase 1 logs: **both `share_accepted` and `share_forwarded`**

**Cautions (still apply in later phases)**
- Don’t call every share a block
- Don’t promise tip/orphan until Phase 4
- Wrong endianness makes “we found a block” unverifiable against explorers/RPC

**Exit criteria**
- [x] Field list + endianness + config knobs agreed and written to the schema doc

---

## Phase 1 — Surface hashes already computed (logs only)

**Status: COMPLETE**

**Goal:** Usable log lines show the header hash for accepts/forwards; `BlockFound` is unmistakable.

**Implemented**
- `Sv1ShareMetadata` + `share_hash` on `SubmitShareWithChannelId` (`src/lib/utils.rs`) threads the SV1 header hash to ChannelManager
- `share_accepted` info log after local MeetsTarget + dedup (`downstream_message_handler.rs`)
- ChannelManager matches `Valid(hash)` vs `BlockFound(hash)`; emits `share_forwarded` (info) and `BLOCK_CANDIDATE` (warn); `share_not_forwarded` (debug) includes SV1 hash when present
- Both `downstream_channel_id` and `upstream_channel_id` on forward/candidate events
- Unit tests: `format_share_hash_uses_display_hex`; full `translator_sv2` lib suite green (107)

**Exit criteria**
- [x] Hex header hash on accept + forward paths
- [x] `BlockFound` / `BLOCK_CANDIDATE` unmistakable

---

## Phase 2 — Durable JSONL audit trail

**Status: COMPLETE** (live validated 2026-10-20 — see `LOG_EVOLUTION.md`)

**Goal:** Records survive log rotation and container restarts.

**Implemented**
- Config: `share_audit_path`, `share_audit_block_candidates_only` ([`config.rs`](src/lib/config.rs))
- Module [`share_audit.rs`](src/lib/share_audit.rs): append-only JSONL, flush per line
- Events: `share_forwarded`, `block_candidate`, `share_upstream_result` (success batched by `last_sequence_number` + count; error by sequence)
- Pending map correlates hash ↔ upstream sequence
- Braiins test config writes `./share-audit-phase2.jsonl`
- Pre-Phase-2 log baseline frozen in [`LOG_EVOLUTION.md`](LOG_EVOLUTION.md)

**Exit criteria**
- [x] Code + unit tests
- [x] Live: JSONL grows on forwards; success/error lines carry matching `share_hash`

---

## Phase 3 — First-class block-candidate handling

**Status: LIVE** (2026-10-21 Braiins session). No `BLOCK_CANDIDATE` yet — expected on mainnet. See [`LOG_EVOLUTION.md`](LOG_EVOLUTION.md).

**Goal:** Treat network-difficulty shares as the rare, high-value event.

**Implemented**
- `BLOCK_CANDIDATE` warn includes `share_hash`, worker, job, sequence, `nbits`, `blocks_found`, and `chain_status=unknown`
- Message states this is **not** a chain-tip confirmation
- Dedicated JSONL `block_candidate_audit_path` (default `block-candidates.jsonl`) always records candidates, even when `share_audit_path` is unset
- Record only — no webhook, no block push
- Unit test: candidate JSONL row carries `nbits` and `chain_status`

**Cautions**
- `BlockFound` means network target met at validation time, not “in best chain”
- Orphans and races are normal; tip check is Phase 4

**Exit criteria**
- [x] Candidate path writes `block_candidate` JSON with hash + nbits (unit test)
- [x] Live process opened `./block-candidates.jsonl` (2026-10-21T18:14:40Z)
- [ ] Live `BLOCK_CANDIDATE` line (only if a share actually meets network difficulty)

---

## Phase 4 — Chain tip / orphan status (Bitcoin)

**Goal:** Answer whether a recorded hash made it into Bitcoin’s best chain.

**Do**
- Optional config: bitcoind RPC URL + auth (or cookie path)
- Background checker for `BLOCK_CANDIDATE` (optionally any audited hash):
  - `getblockheader` / `getblock`
  - missing → `not_found` (yet)
  - in best chain → `in_best_chain` + confirmations
  - known but not best chain → `orphaned_or_stale` (careful wording)
- Append `chain_status` events to JSONL
- Poll with backoff; stop after N confirmations or TTL

**Questions (must answer before coding)**
- Local/full bitcoind RPC available, or defer this phase?
- Mainnet only, or also testnet/signet labels?
- How long to poll a missing candidate (hours vs days)?

**Cautions**
- Never commit RPC credentials; prefer cookie auth
- Don’t mark orphan too early (propagation delay)
- Trust boundary = whatever bitcoind you point at
- Rate-limit RPC; don’t check every ordinary share

**Exit criteria**
- Known fixture/mainnet hash → `in_best_chain` with confirmations
- Synthetic unknown hash → `not_found`, then ages out

---

## Phase 5 — Hardening and ops handoff

**Goal:** Safe to run beside real miners.

**Do**
- Config examples under `config-examples/`
- README: log meanings, JSONL schema, grepping `BLOCK_CANDIDATE`, tip-check usage
- Load/sanity: confirm info-level volume; document recommended levels
- Tests for aggregated vs non-aggregated correlation
- Decide upstream PR to `sv2-apps` vs private fork

**Questions**
- Ship as PR to `sv2-apps`, or private ops fork first?
- Retention / redaction rules for worker names?

**Cautions**
- Don’t hurt share hot-path latency
- Config-gate JSONL + RPC so default behavior stays clean for upstream PRs
- Keep NTP sync (existing cert validation warning still applies); timestamps matter for incidents

**Exit criteria**
- Documented, config-gated feature usable on Bitcoin without reconstructing from `mining.submit` alone

---

## Build order

| Phase | Delivers | Depends on |
|-------|----------|------------|
| 0 | Shared vocabulary | — |
| 1 | Hashes in logs + `BlockFound` visible | 0 |
| 2 | JSONL audit | 1 |
| 3 | Block-candidate prominence | 1–2 |
| 4 | Tip/orphan via bitcoind | 2–3 + RPC access |
| 5 | Prod-ready docs/tests | 1–4 |

---

## Decisions log

| # | Decision | Answer |
|---|----------|--------|
| 1 | Hash display bitcoind/explorer hex | **Yes** (Phase 0) |
| 2 | JSONL via `share_audit_path` | **Yes** (Phase 0) |
| 3 | Phase 1 log both accepted + forwarded | **Yes** |
| 4 | Farm-scale volume for those levels | **OK** |
| 5 | bitcoind RPC for Phase 4 now, or defer? | *open* |
| 6 | Private fork first vs PR-ready to sv2-apps? | *open* |
| 7 | Phase 0 schema frozen | **Yes** — see `SHARE_HASH_AUDIT_SCHEMA.md` |
| 8 | Phase 1 structured share-hash logs | **Yes** |
| 9 | Phase 1 live validated (Braiins + miner) | **Yes** (2026-10-20) — see `PHASE1_CHANGES.md` |
| 10 | Phase 2 JSONL implemented + live validated | **Yes** (2026-10-20) — see `LOG_EVOLUTION.md` |
| 11 | Phase 3 block-candidate file + nbits | **Yes, live** (2026-10-21) — file opened; 0 candidates. Ordinary nacks no longer appended there after the `share_audit` fix |

---

## What exists today vs this plan

| Capability | Today | After plan |
|------------|-------|------------|
| Compute share header hash | Yes (validation/dedup) | Same, but surfaced |
| Hash in normal info logs | No (debug only) | Yes (Phase 1) |
| Distinct `BlockFound` handling | Counter only; Valid/BlockFound treated alike in app logs | Loud candidate event (Phases 1 + 3) |
| Per-share durable history | No | JSONL (Phase 2) |
| Chain tip / orphan | No | bitcoind RPC (Phase 4) |
| Monitoring per-share history API | No | Out of scope for v1 |
