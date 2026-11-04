use futures_util::StreamExt;
use solana_pubsub_client::nonblocking::pubsub_client::{self, PubsubClient};
use solana_rpc_client_api::response::SlotUpdate::FirstShredReceived;
use solana_tpu_client_next::leader_updater::LeaderUpdater;

use crate::{refresh_leader_schedule, state::MY_LEADER_SCHEDULE};

pub fn get_current_leader() {}

pub struct ScheduleLeaderUpdater;

#[async_trait::async_trait]
impl LeaderUpdater for ScheduleLeaderUpdater {
    fn next_leaders(&mut self, lookahead_leaders: usize) -> Vec<std::net::SocketAddr> {
        let initial_schedule = &MY_LEADER_SCHEDULE;
        let schedule = initial_schedule
            .read()
            .expect("leader schedule lock poisoned");

        let Some(offset) = schedule
            .current_slot
            .checked_sub(schedule.epoch_start_slot)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return Vec::new();
        };

        let schedule_len = schedule.leader_schedule_at_offset.len();
        eprintln!(
            "leader lookup state: current={}, epoch_start={}, offset={}, schedule_len={}",
            schedule.current_slot, schedule.epoch_start_slot, offset, schedule_len,
        );

        if offset >= schedule_len {
            eprintln!("leader lookup cannot start: offset is outside this epoch schedule");
            return Vec::new();
        }

        let mut result = Vec::with_capacity(lookahead_leaders);

        let mut missing_validator_count = 0usize;
        let mut missing_quic_count = 0usize;
        let mut scanned_slots = 0usize;

        for (relative_offset, &leader_id) in schedule
            .leader_schedule_at_offset
            .iter()
            .skip(offset)
            .enumerate()
        {
            scanned_slots += 1;
            let Some(validator) = schedule.leader_tpu_at_offset_index.get(leader_id as usize)
            else {
                missing_validator_count += 1;
                if missing_validator_count <= 3 {
                    eprintln!(
                        "leader schedule offset {} has ID {}, but there are only {} validator entries",
                        offset + relative_offset,
                        leader_id,
                        schedule.leader_tpu_at_offset_index.len(),
                    );
                }
                continue;
            };

            let Some(quic_socket) = validator.tpu_quic else {
                missing_quic_count += 1;
                if missing_quic_count <= 3 {
                    eprintln!(
                        "leader schedule offset {} resolves to {}, but it has no TPU/QUIC address",
                        offset + relative_offset,
                        validator.address,
                    );
                }
                continue;
            };

            // A leader normally owns four consecutive slots.
            // Return it once, then continue to the next distinct leader.
            if result.last() == Some(&quic_socket) {
                continue;
            }

            result.push(quic_socket);

            if result.len() == lookahead_leaders {
                break;
            }
        }

        if result.len() < lookahead_leaders {
            eprintln!(
                "leader lookup returned {} endpoints after scanning {} slots (missing validator: {}, missing QUIC: {})",
                result.len(),
                scanned_slots,
                missing_validator_count,
                missing_quic_count,
            );
        }

        result
    }

    async fn stop(&mut self) {
        // State is owned by the application, so there is no worker to stop here.
    }
}

pub async fn track_slots_update(websocket: String) {
    let client = PubsubClient::new(websocket)
        .await
        .expect("failed to establish websocket connection");

    println!("connecting to slot-update WebSocket: ");
    let (mut events, unsubscribe) = client
        .slot_updates_subscribe()
        .await
        .expect("subscribing to slot update failed");

    println!("slot-update subscription active");

    while let Some(event) = events.next().await {
        match event {
            FirstShredReceived { slot, .. } => {
                // println!("slot-update ..... {slot}");
                let should_refresh_epoch = {
                    let schedule = MY_LEADER_SCHEDULE
                        .read()
                        .expect("leader schedule lock poisoned");

                    let epoch_end =
                        schedule.epoch_start_slot + schedule.leader_schedule_at_offset.len() as u64;

                    slot >= epoch_end
                };

                if should_refresh_epoch {
                    println!("epoch boundary reached at slot {slot}; refreshing schedule");

                    refresh_leader_schedule().await; // or just push to a tokio::spawn?
                } else {
                    let mut schedule = MY_LEADER_SCHEDULE
                        .write()
                        .expect("leader schedule lock poisoned");

                    // Ignore an older event after a reconnect/reordering.
                    if slot > schedule.current_slot {
                        schedule.current_slot = slot;
                    }
                }
            }
            _ => {
                continue;
            }
        }
    }
}
