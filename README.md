# nik8 / communik8

Push-to-talk voice over Solana.

Hold Push on one phone, speak, and a nearby phone plays it. The audio travels as
Opus packets inside Solana transactions.

> **Status: demo.** Runs on devnet or testnet. The app bundles its gRPC token
> (see [Limits](#limits)), so don't hand the APK to people you don't trust.

---

## How it works

```
 PHONE A (sender)                                       PHONE B (receiver)
 ───────────────                                        ──────────────────
 mic 16 kHz
   │
 Opus, 120 ms per packet
   │
 signed by the device's
 session key
   │
   ├─▶ WebTransport ─▶ sender (server) ─▶ leader TPUs ─┐
   │                                                   │
   └─▶ JSON-RPC (fallback) ──────────────▶ RPC node ───┤
                                                       ▼
                                          Nik8 program (on-chain)
                                                       │
                                          Yellowstone gRPC stream
                                                       │
                                                       ▼
                                          filter: recipient == me
                                          dedupe, reorder, decode
                                                       │
                                                    speaker


 Discovery: each phone advertises its session address over Bluetooth LE.
 Nearby phones show up as bubbles. Tap one to talk to it.
```

### The pieces

1. **Session key.** The app makes its own keypair on first launch and stores
   it encrypted with the Android Keystore. Your wallet funds it once. It then
   signs every packet, so there's no wallet popup per packet.
2. **Sending.** Each voice transaction holds one or more `OnewayMessage`
   instructions. Each instruction is one 120 ms Opus packet.
   - Through `sender`: 1 packet per transaction. `sender` forwards straight to
     the next leaders.
   - Through RPC: 2 packets per transaction, because public RPC allows about
     4 sends per second.
3. **Receiving.** The app subscribes to Yellowstone for Nik8 transactions. It
   keeps packets addressed to its session key and plays them.
4. **Discovery.** Bluetooth LE, no connection. The 32-byte address is split
   across one advertisement (first 26 bytes) and its scan response (last 6).

### Voice packet (v1)

Instruction data for `OnewayMessage`:

| Bytes | Field |
| --- | --- |
| 1 | discriminator = `1` |
| 1 | version = `1` |
| 1 | codec = `1` (Opus) |
| 16 | session ID (new per Push) |
| 32 | recipient address |
| 4 | sequence, u32 LE |
| 8 | capture timestamp ms, u64 LE |
| ≤ 1275 | Opus packet |

Full program interface: [`program/INTERFACE.md`](program/INTERFACE.md).

### Measured latency

Capture → arrival on Yellowstone, devnet, 4 s of speech, run from a desktop:

| Path | p50 | p95 |
| --- | --- | --- |
| RPC | ~700 ms | ~1,000 ms |
| `sender` ingress | ~560 ms | ~720 ms |

Playback buffers up to ~240 ms on top, to smooth out arrival timing.

---

## Repo layout

| Path | What it is |
| --- | --- |
| `program/` | On-chain program (Pinocchio). |
| `shared/` | Voice packet format. `no_std`. Used by the program and clients. |
| `consumer/` | Yellowstone subscription with reconnect, plus a CLI that prints packets. |
| `nik8-core/` | Client logic: transactions, Opus, jitter buffer, listener, push-to-talk, ingress client. |
| `nik8-ffi/` | Exposes `nik8-core` to Kotlin through UniFFI. Built as an Android `.so`. |
| `sender/` | Server. WebTransport ingress → leader TPUs. Tracks the leader schedule. |
| `nik8-mobile/` | Expo / React Native app. Native code is in `modules/nik8-native/`. |
| `scripts/build-android-rust.sh` | Builds `nik8-ffi` for Android and generates the Kotlin bindings. |

The program ID is `NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU` on devnet and testnet.

---

## Setup

### You need

- Rust (stable) with the Android targets:
  `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`
- Android SDK + NDK (`ANDROID_HOME` set)
- Node and pnpm
- Solana CLI (for keypairs and airdrops)
- An Android phone. The app is Android only.

### 1. Config

```bash
cp .env.example .env
ln -s ../.env nik8-mobile/.env
```

Fill in `.env`. Every value has a comment saying where to get it. At minimum:

- `RPC_URL`
- `NIK8_GRPC_ENDPOINT` and `NIK8_GRPC_X_TOKEN` (same cluster as `RPC_URL`)

### 2. Build the Rust library for Android

```bash
./scripts/build-android-rust.sh arm64-v8a
```

Run this again whenever Rust code changes. Its output isn't committed.

### 3. Run the app

```bash
cd nik8-mobile
pnpm install
pnpm android        # builds, installs on the connected phone, starts Metro
```

Standalone APK (no Metro needed on the phone):

```bash
pnpm android:release
adb install -r android/app/build/outputs/apk/release/app-release.apk
```

### 4. In the app

1. **Activ8 comms** → approve the wallet.
2. **Fund session** → one wallet popup, sends 0.05 SOL to the session key.
3. **Register sender** → no popup.
4. Once Online, nearby phones appear as bubbles. Tap one.
5. Hold **Push** and talk.

With no peer selected, Push talks to yourself (loopback). Useful for testing on one phone.

---

## Running the server (optional)

Without `sender`, the app sends through RPC. With it, sends are faster.

```bash
cd sender
cargo run -p nik8_sender
```

Then set `NIK8_INGRESS_URL` and `NIK8_INGRESS_CERT_SHA256` in `.env`. The
commands for the certificate are in `.env.example`. Rebuild the app after.

The phone must reach the server on **UDP 4433**.

---

## Tests

```bash
cargo test --workspace --exclude nik8_program
```

Live tests send real transactions (they cost a few thousand lamports). They need
a funded `PAYER_KEYPAIR`. The first test registers it as a sender if needed.

```bash
cargo test -p nik8_core --test testnet -- --ignored --nocapture
```

| Test | What it checks |
| --- | --- |
| `registers_and_sends_a_voice_packet_to_self` | One packet end to end. |
| `round_trips_audio_through_the_chain` | `voice-hello-there.wav` → chain → decoded WAV in `target/`. |
| `measures_packet_latency` | p50 / p95 / max latency. `NIK8_SUBMIT=rpc` forces the RPC path. |

Watch packets from the command line:

```bash
cargo run -p nik8_consumer
```

---

## Limits

- **Android only.** iOS needs a different wallet connection and BLE advertising.
- **Foreground only.** Android cuts network access for background apps.
- **Audio is public.** Anyone reading the chain can decode it. There's no encryption yet.
- **The gRPC token ships in the APK.** `EXPO_PUBLIC_*` values can be extracted.
  The fix is a gateway server that holds the token. Not built yet.
- **The `sender` ingress has no auth yet.** It only accepts valid Nik8
  transactions. Add session-key auth and rate limits before exposing it publicly.
- **Bluetooth distance is rough.** Treat it as near / medium / far.
- **Release builds use the template debug keystore.** Fine for sideloading only.

---

## More

- [`PROPOSAL.md`](PROPOSAL.md): planned protocol changes (multi-frame packets, delegated vault).
- [`program/INTERFACE.md`](program/INTERFACE.md): instructions, accounts, packet format.
