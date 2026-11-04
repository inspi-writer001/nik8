# Nik8 Protocol Proposals

**Status:** Brainstorm / not scheduled  
**Last updated:** 2026-10-29  
**Scope:** Changes that would require a new wire version, a program upgrade, or both. Anything here is _planned_, not implemented. The baseline below is what we are building against today.

## 0. Current baseline (decided)

| Area                | Decision                                                                                                                                                                                                                                       |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Batching            | **120 ms of audio per transaction**, carried as a single Opus packet (e.g. 2 × 60 ms frames, Opus code 3) inside one v1 `OnewayMessage` instruction. No wire change.                                                                            |
| Wire format         | v1 unchanged: `[disc=1][version=1][codec=1][session_id 16][recipient 32][sequence u32 LE][capture_timestamp_ms u64 LE][opus ≤ 1275]`. `sequence` and `capture_timestamp_ms` now count **batches**, not 20 ms frames.                          |
| Signing             | Local session keypair generated on device, encrypted at rest, registered as the `sender` via `Cre8Account`. The connected wallet funds it once. No per-packet wallet prompt.                                                                    |
| Balance             | Session key holds a small SOL float (network fees + per-packet `fee_amount`). The app offers "withdraw to wallet", signed locally. Funds on the device are at risk if the device or app data is lost, so the float is kept small and topped up. |
| Recipient filtering | Recipient stays in instruction data only. Receivers filter locally on `header.recipient`; Yellowstone filters by program only.                                                                                                                 |

v1 transaction size at 120 ms / 16 kbps (legacy message, 1 signature, 5 account keys incl. program):

```
signatures      1 + 64      =  65
message header              =   3
account keys    1 + 5 × 32  = 161
recent blockhash            =  32
ix count                    =   1
ix: program idx 1, accounts 1+4, data len 2, data 1+62  =  71
----------------------------------------------------------
fixed overhead              ≈ 333 bytes
opus (120 ms @ 16 kbps)     ≈ 240 bytes
total                       ≈ 573 / 1,232 bytes
```

Adding priority fees (Compute Budget program key + 2 instructions) costs roughly another 50 bytes.

---

## P1. Voice packet v2: multi-frame batches (up to ~300 ms per instruction)

**Status:** Proposed  
**Touches:** `shared`, `program` (upgrade), `consumer`, mobile core

### Problem

An Opus packet can carry at most **120 ms** of audio, and v1 allows exactly one Opus packet per instruction. To go past 120 ms per transaction today we would need multiple `OnewayMessage` instructions, which means:

- one `fee_amount` transfer per instruction (fees scale with instruction count, not audio),
- ~71 bytes of repeated instruction + header overhead per extra instruction,
- multiple sequence numbers for what is logically one batch.

Push-to-talk speech doesn't need 20 ms or even 120 ms delivery granularity. 150–300 ms per transaction cuts transaction count and base fees substantially.

### Proposed wire format (version = 2)

The 62-byte `VoicePacketHeader` layout is unchanged; only `version` differs and the body becomes a frame list.

```
Instruction data:
  [discriminator: u8 = 1]                  // OnewayMessage, unchanged
  [version: u8 = 2]
  [codec: u8 = 1 (Opus)]
  [session_id: 16 bytes]
  [recipient: 32 bytes]
  [sequence: u32 LE]                       // per batch
  [capture_timestamp_ms: u64 LE]           // capture time of the first frame
  [frame_count: u8]                        // 1..=MAX_FRAMES_PER_BATCH
  repeated frame_count times:
    [len: u16 LE]                          // 1..=1275
    [opus_packet: len bytes]
```

Validation rules (shared parser, used by program and receivers):

- `frame_count >= 1` and `<= MAX_FRAMES_PER_BATCH` (proposed: 15, enough for 300 ms of 20 ms frames).
- Every `len` is in `1..=1275`.
- Frame lengths must sum exactly to the remaining data; **no trailing bytes**.
- Receivers additionally decode each Opus TOC byte and reject batches whose total duration exceeds `MAX_BATCH_MS` (proposed: 300). The program only checks structure; TOC duration parsing is cheap but not required on-chain.

Alternative considered: Opus self-delimiting framing (RFC 6716 Appendix B) instead of `u16` length prefixes. It saves a byte per frame but is harder to read and validate; `u16` is preferred for clarity.

### Size budget

Fixed overhead stays ≈ 333 bytes, plus 1 byte `frame_count` and 2 bytes per frame.

| Batch  | Frames       | Bitrate | Opus bytes | Total  | Fits 1,232? |
| ------ | ------------ | ------- | ---------- | ------ | ----------- |
| 300 ms | 5 × 60 ms    | 16 kbps | ~600       | ~944   | Yes         |
| 300 ms | 15 × 20 ms   | 16 kbps | ~600       | ~964   | Yes         |
| 300 ms | 5 × 60 ms    | 24 kbps | ~900       | ~1,244 | **No**      |
| 240 ms | 4 × 60 ms    | 24 kbps | ~720       | ~1,062 | Yes         |

At 300 ms, bitrate is effectively capped around 20 kbps. Senders must check the serialized size before signing and fall back to a shorter batch if needed (VBR packets vary).

### Semantics

- `sequence` increments once per batch.
- Frame `i` capture time = `capture_timestamp_ms + Σ duration(frames[0..i])`, derived from Opus TOC.
- One `fee_amount` per instruction, as today. Open question: should the fee scale with duration or bytes in v2?

### Trade-offs

- **Latency:** the receiver can't play a batch until the whole transaction lands, so batch length adds directly to mouth-to-ear latency (300 ms batch + chain inclusion + Yellowstone delivery + jitter buffer).
- **Loss:** one dropped transaction = a 300 ms gap. Opus PLC won't conceal that convincingly. Possible later mitigation: a low-bitrate redundant copy of the previous batch's last frame (costs bytes).
- **Jitter buffer** on the receiver must hold at least one full batch.

### Rollout

1. `shared`: add v2 parsing alongside v1 (`VoicePacket` exposes a frame iterator; v1 = a single frame). Add a v2 builder.
2. Receivers / `consumer`: accept v1 and v2. Ship this first so nothing in the field rejects v2.
3. `program`: `process_messenger` currently calls `VoicePacket::parse`, which rejects `version != 1`. Upgrade the program to accept v2 (requires the upgrade authority for `NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU`).
4. Senders switch to v2 once the program upgrade is confirmed on the target cluster.
5. Benchmark 120 / 180 / 240 / 300 ms: fees per minute, p50/p95 mouth-to-ear latency, audible gap rate.

---

## P2. Delegated sender vault (wallet authority + device delegate)

**Status:** Proposed  
**Touches:** `program` (new state + instructions, breaking for `messenger`), mobile core, setup flow

### Problem

The baseline keeps the spendable balance on the device's session keypair. Solana has **no native delegate for SOL**: a system-owned account can only be debited by its own signature. So:

- Transferring authority of the session account to the user's wallet defeats the purpose; every packet would then need a wallet popup.
- Keeping the authority on the device means the wallet has no on-chain power over those funds. A lost phone or wiped app = lost float.

What we want: **the device key signs every packet without prompts, and the wallet keeps ultimate control of the money.** That only works if the balance lives in a **program-owned account** whose rules the Nik8 program enforces.

### Proposed state

New PDA, one per wallet, seeds `[b"nik8_vault", authority]`:

```rust
#[repr(C)]
pub struct SenderVault {
    pub bump: u8,
    /// Connected wallet. Full control: deposit, withdraw, set/revoke delegate.
    pub authority: [u8; 32],
    /// Device session key. May only sign OnewayMessage and draw capped gas.
    /// All zeroes = no delegate (revoked).
    pub delegate: [u8; 32],
    /// Optional expiry for the delegate (unix seconds, 0 = none).
    pub delegate_expires_at: [u8; 8],
    /// Packet fees owed to the protocol, not yet swept by the admin.
    pub accrued_fees: [u8; 8],
    /// Max lamports the delegate may pull for network fees per top-up window.
    pub gas_allowance: [u8; 8],
}
```

Accounting inside the vault:

```
spendable = lamports − rent_exempt_minimum − accrued_fees
```

`messenger` would **increment `accrued_fees`** instead of doing a system-program transfer, so no CPI and no extra writable account per packet. The admin sweep only ever takes `accrued_fees`; the authority withdraw only ever takes `spendable`. Neither can touch the other's share.

### Proposed instructions

| Instruction                | Signer             | Effect                                                                                                                                                                     |
| -------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `InitVault`                | authority (wallet) | Create vault, set `delegate`, optional expiry and gas allowance, deposit initial lamports. **The one wallet popup during setup.**                                           |
| `Deposit`                  | anyone             | Add lamports.                                                                                                                                                              |
| `SetDelegate`              | authority          | Rotate or revoke (zero) the delegate, update expiry/allowance. Used on device loss or reinstall.                                                                           |
| `Withdraw`                 | authority          | Move up to `spendable` to any destination.                                                                                                                                 |
| `WithdrawToAuthority`      | delegate           | Move up to `spendable` **only to `authority`**. Lets the app offer "send balance back to my wallet" with no popup and no risk of the delegate draining elsewhere.          |
| `TopUpDelegateGas`         | delegate           | Move up to `gas_allowance` from the vault to the delegate's system account so it can keep paying network base fees. Rate limited (per slot window).                          |
| `OnewayMessage` (modified) | delegate           | Verify `delegate` matches signer and is not expired, check `spendable ≥ fee_amount`, add `fee_amount` to `accrued_fees`, then validate the voice packet as today.          |
| `AdminWithdrawFee` (mod.)  | fee authority      | Sweep up to `accrued_fees` and decrement it.                                                                                                                               |

### Limitation that remains

The **transaction fee payer must be a system-owned signer**, so the delegate still needs a small SOL balance for the 5,000-lamport base fee (plus any priority fee). The program cannot pay base fees from a PDA. `TopUpDelegateGas` keeps this automatic and bounded; worst case on device loss is the delegate's gas float, not the whole vault.

### Setup flow with P2

1. App generates the session keypair locally and stores it encrypted (Keystore / Keychain).
2. User connects wallet.
3. Wallet signs **one** transaction: `InitVault(delegate = session pubkey, deposit = X)` + a small SOL transfer to the session key for initial gas.
4. From then on, the session key signs every `OnewayMessage` and gas top-up. No prompts.
5. On a new device: wallet signs `SetDelegate(new key)`. Old key is dead immediately.

### Migration from the baseline

- Existing `SenderAccount` (`[b"nik8_user", sender]`, admin-swept fee vault) and the current `messenger` account list are incompatible with this design. Either:
  - add the vault path as a new instruction discriminator and keep v1 `OnewayMessage` working during transition, or
  - redeploy and migrate in one step (acceptable while there are no real users).
- Mobile core must support both signing modes until the old path is retired.

### Open questions

- Per-day spend caps on the delegate in addition to expiry?
- Should `gas_allowance` be a per-window cap or a lifetime cap reset by the authority?
- Is one vault per wallet enough, or do we want one per (wallet, device) pair?

---

## P3. Recipient as a read-only transaction account (server-side filtering)

**Status:** Declined for now (2026-10-29)

Idea: add the recipient address as an extra read-only account on `OnewayMessage` (the program already ignores trailing accounts via `_others @ ..`). Receivers could then subscribe with `account_include: [my_address]` + `account_required: [program]` and Yellowstone would only stream their own audio.

Why declined: the recipient is already in the header, and the extra account costs 32 bytes per transaction for duplicated information. We accept that Yellowstone can't filter on instruction data, so every receiver streams every Nik8 transaction and filters locally.

Revisit if: concurrent users grow to the point where per-device bandwidth, battery or provider egress costs from unfiltered streams become measurable problems.
