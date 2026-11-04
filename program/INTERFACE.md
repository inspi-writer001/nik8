# Nik8 program interface (draft v0)

This document locks the instruction discriminators, account ordering, and
voice-packet byte format for the sender-side program. `VoicePacketHeader` is
a 62-byte `repr(C)` bytemuck struct whose fields are byte arrays, so its Rust
layout is exactly its wire layout: no integer-alignment padding is involved.

All public keys are 32-byte Solana addresses. `system_program` must be the
canonical System Program and is verified by the relevant implementation.

| Discriminator | Instruction | Accounts, in order | Instruction data |
| --- | --- | --- | --- |
| `0` | `Initialize` | `0. configuration_authority` (signer, writable payer); `1. global_config` (writable PDA); `2. system_program` | `fee_lamports: u64 LE` followed by `fee_withdraw_authority: [u8; 32]` |
| `1` | `OnewayMessage` | `0. sender` (signer, writable); `1. sender_fee_vault` (writable PDA); `2. global_config` (read-only PDA); `3. system_program` | One `VoicePacket`, described below |
| `2` | `Cre8Account` | `0. sender` (signer, writable payer); `1. sender_fee_vault` (writable PDA); `2. system_program` | Empty |
| `3` | `AdminWithdrawFee` | `0. fee_withdraw_authority` (signer); `1. global_config` (read-only PDA); `2. sender_fee_vault` (writable PDA); `3. destination` (writable) | `lamports: u64 LE` |
| `4` | `UpdatePacketFee` | `0. configuration_authority` (signer); `1. global_config` (writable) | `new_fee_lamports: u64 LE` |

## PDA derivations

```text
global_config  = PDA(["nik8_config"], program_id)
sender_fee_vault = PDA(["nik8_user", sender_pubkey], program_id)
```

`sender_fee_vault` is persistent fee state only. It never stores voice
packets. The recipient is intentionally not an instruction account: receivers
subscribe to transactions invoking this program and route packets by the
recipient address and session metadata in instruction data.

## VoicePacket v1 (`OnewayMessage` data)

```text
offset  size  field
0       1     version = 1
1       1     codec = 1 (Opus)
2       16    session_id (opaque, client-generated)
18      32    recipient: [u8; 32] (Solana address)
50      4     sequence_le: [u8; 4] (u32 little-endian; starts at 0 for each sender/session)
54      8     capture_timestamp_ms_le: [u8; 8] (u64 little-endian; start of this audio packet)
62      N     one complete Opus packet
```

The trailing Opus packet carries **100 ms** of audio, normally five 20 ms
frames encoded as one standard Opus packet. At 16 kbps that is approximately
200 bytes. Its variable byte length is already represented by the instruction
data length, so Nik8 adds no per-frame length fields. It must be non-empty and
at most 1,275 bytes, the Opus packet limit.

## Current execution status

The entrypoint dispatches all four discriminators. `Cre8Account` creates the
exact sender vault PDA; `OnewayMessage` validates both that vault and the
singleton config PDA before transferring its configured packet fee.
`AdminWithdrawFee` requires the configured withdrawal authority, validates
the exact sender vault, and permits only a nonzero amount that leaves the
vault rent-exempt. `UpdatePacketFee` requires the configuration authority
stored in `GlobalConfig` and changes the fee for future voice packets.
