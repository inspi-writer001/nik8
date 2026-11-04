# Consumer Resilience Suggestions

**Status:** Suggested, not applied  
**Touches:** `consumer/src/nik8_instruction.rs`, `consumer/src/yellowstone.rs`, `consumer/src/main.rs`  
**Used by:** desktop CLI and `nik8-mobile` receive path

## 1. Skip, don't abort, on bad data

### Problem

Every `?` in the receive path ends the consumer:

| Location                         | Error                                                        | Effect                                                                           |
| -------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| `nik8_instruction.rs:104`        | `VoicePacket::parse` fails on one Nik8 instruction            | Whole update returns `Err`; valid packets already collected in that tx are lost. |
| `nik8_instruction.rs:76`         | `ProgramIdIndexOutOfBounds` on any instruction                | Same.                                                                            |
| `nik8_instruction.rs:49`, `:65`, `:70` | `MissingTransaction` / `MissingMessage` (malformed update) | Same.                                                                            |
| `main.rs:13`                     | `extract_voice_packets(&update)?`                             | Any of the above exits the process and drops the subscription.                   |

Unreachable while program and consumer share one parser version, because the program rejects invalid packets and failed transactions are filtered out. It becomes reachable on a version rollout (a v1-only receiver seeing v2), a different program deployment, or malformed provider data.

### Fix

- One bad instruction never affects other instructions; one bad update never affects the stream.
- `extract_voice_packets` returns packets plus rejections instead of `Result`:

  ```rust
  pub struct Extracted<'a> {
      pub packets: Vec<VoicePacket<'a>>,
      pub rejected: Vec<Nik8InstructionError>,
  }

  pub fn extract_voice_packets(update: &SubscribeUpdate, program_id: &[u8; 32]) -> Extracted<'_>;
  ```

  - Transaction-level problems (`MissingTransaction`, `MissingMessage`): return no packets, one rejection.
  - Instruction-level problems (`ProgramIdIndexOutOfBounds`, `InvalidVoicePacket`): record the rejection, `continue` to the next instruction.
- Distinguish `UnsupportedFormat` (unknown version or codec) from malformed data in metrics. A rising unsupported count means receivers need upgrading, not that the data is bad.
- Remove the `println!("received transaction")` debug line.
- Take the program ID as a parameter instead of reading `NIK8_PROGRAM_ID_BYTES`.
- Callers count and log rejections (rate-limited) and keep going. `main.rs` drops the `?`.

### Tests

- Transaction with one valid and one invalid Nik8 instruction → one packet, one rejection.
- Unknown version → rejected as unsupported, not malformed.
- Out-of-bounds program ID index → that instruction skipped, others kept.
- Update with no transaction / no message → empty result, no panic.
- Non-transaction updates (ping, slot) → empty result, no rejection.

## 2. Reconnect instead of exit

### Problem

```rust
while let Some(update) = stream.next().await {
    let update = update?;                          // stream error → process exits
    ...
}
anyhow::bail!("Nik8 Yellowstone stream closed")    // clean close → process exits
```

Long-lived gRPC streams break routinely: network switches, provider restarts, load balancer idle timeouts, unanswered pings, silent stalls. `subscribe_once` returns only the receive half, so the client cannot answer provider pings. A stalled stream (connection open, no data) is never detected.

### Fix

1. **Reconnect loop.** Outer loop around connect → subscribe → read. Stream error or close → back off and reconnect.
2. **Backoff.** Exponential from ~250 ms, capped at 15–30 s, with jitter. Reset after a stream stays healthy (~30 s). Classify errors:
   - fatal (`Unauthenticated`, `PermissionDenied`, invalid config) → stop and report;
   - transient (`Unavailable`, transport, timeout) → retry.
3. **Keepalive and stall detection.**
   - Use bidirectional `subscribe` (sink + stream); answer each `Ping` update with `SubscribeRequest { ping: Some(..) }`.
   - Enable HTTP/2 / TCP keepalive and a connect timeout on the client builder.
   - Wrap `stream.next()` in `tokio::time::timeout`; no message (including pings) within the stall window → treat as dead and reconnect.
4. **After reconnect.**
   - Resume live. Do not replay the gap via `from_slot`; stale voice is useless.
   - Reset jitter buffers; treat the sequence gap as loss.
5. **Lifecycle.**
   - Cancellation token so stop/pause ends the loop immediately, including during backoff sleeps.
   - State events: `Connecting`, `Live`, `Reconnecting { attempt }`, `Failed { reason }`.

### Shape

```rust
pub async fn run_subscription(
    config: SubscriptionConfig,
    cancel: CancellationToken,
    on_event: impl Fn(SubscriptionEvent),
    on_update: impl FnMut(&SubscribeUpdate),
) -> Result<(), FatalError> {
    let mut backoff = Backoff::new();
    loop {
        let (mut sink, mut stream) = match connect_and_subscribe(&config).await {
            Ok(pair) => { on_event(Live); pair }
            Err(e) if is_fatal(&e) => { on_event(Failed(&e)); return Err(e.into()); }
            Err(_) => { on_event(Reconnecting(backoff.attempt())); sleep_or_cancel(&cancel, backoff.next()).await?; continue; }
        };

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                next = timeout(config.stall_after, stream.next()) => match next {
                    Err(_) | Ok(None) | Ok(Some(Err(_))) => break,
                    Ok(Some(Ok(update))) => {
                        backoff.mark_healthy();
                        if is_ping(&update) { let _ = sink.send(pong()).await; continue; }
                        on_update(&update);
                    }
                }
            }
        }

        on_event(Reconnecting(backoff.attempt()));
        sleep_or_cancel(&cancel, backoff.next()).await?;
    }
}
```

Lives in `consumer/src/yellowstone.rs`; the CLI and `nik8-mobile` both call it.

Verify builder keepalive method names and the ping request type against the pinned `yellowstone-grpc-client` 16 / `yellowstone-grpc-proto` 14 before implementing.

### Tests

- Mock stream that errors mid-way → reconnects, emits `Reconnecting` then `Live`.
- Mock stream that goes silent → stall timeout triggers reconnect.
- Auth failure → `Failed`, no retry loop.
- Cancel during backoff → returns promptly.
- Ping update → pong sent, not passed to `on_update`.
