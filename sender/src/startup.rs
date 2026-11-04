use std::sync::Arc;

use crate::{
    get_cluster_nodes, get_epoch_info, get_leader_schedule,
    state::{MY_LEADER_SCHEDULE, Validator},
    write_leaders_to_file, write_ports_to_file,
};

pub async fn refresh_leader_schedule() {
    // fetch leader schedule for that epoch and update MY_LEADER_SCHEDULE ?

    // Use one RPC snapshot for both the epoch start and current slot. Fetching
    // them independently can produce an offset from a different epoch view.
    let current_epoch = get_epoch_info().await.expect("couldn't fetch epoch info");
    let current_slot = current_epoch.absolute_slot;
    println!(
        "epoch {}: current slot {}, epoch offset {}/{}",
        current_epoch.epoch, current_slot, current_epoch.slot_index, current_epoch.slots_in_epoch,
    );

    let schedule_leaders = Arc::new(
        get_leader_schedule(current_slot)
            .await
            .expect("scheduled leaders error"),
    );

    let first_clone = schedule_leaders.clone();

    let cluster_info = get_cluster_nodes()
        .await
        .expect("couldn't get cluster_info");
    let list_of_quic_address: Arc<Vec<Validator>> = Arc::new(
        cluster_info
            .iter()
            .filter_map(|node| {
                let validator_info = Validator {
                    address: node.pubkey.clone(),
                    rpc: node.rpc,
                    tpu_quic: node.tpu_quic,
                    tpu_udp: node.tpu,
                };
                Some(validator_info)
            })
            .collect(),
    );

    let nodes_clone = list_of_quic_address.clone();

    tokio::spawn(async move {
        match write_leaders_to_file(current_epoch.epoch, first_clone).await {
            Ok(path) => println!("wrote leader schedule to {}", path.display()),
            Err(error) => println!("failed to write leader schedule: {error}"),
        }
    });

    tokio::spawn(async move {
        match write_ports_to_file(current_epoch.epoch, nodes_clone).await {
            Ok(path) => println!("wrote leader schedule to {}", path.display()),
            Err(error) => println!("failed to write leader schedule: {error}"),
        }
    });

    let global_schedule = &MY_LEADER_SCHEDULE;

    let mut slot_leaders_ordered = vec![u16::MAX; current_epoch.slots_in_epoch as usize];
    let mut leaders_in_index = Vec::with_capacity(schedule_leaders.len());
    let mut leaders_node_config_in_index = vec![Validator::default(); schedule_leaders.len()];

    // expensive

    // for (address, slot_offsets) in schedule_leaders.iter() {
    //     for val in slot_offsets {
    //         big_array[*val] = {
    //             let key_of_address = schedule_leaders.keys().position(|addr| addr == address);

    //             if let Some(index) = key_of_address {
    //                 index as u16
    //             } else {
    //                 9999u16
    //             }
    //         }
    //     }
    // }

    // more efficient
    for (index, (address, slot_offsets)) in schedule_leaders.iter().enumerate() {
        let validator_id_in_offset = u16::try_from(index).expect("too many validators");
        leaders_in_index.push(address.clone());
        for val in slot_offsets {
            slot_leaders_ordered[*val] = validator_id_in_offset
        }
    }

    // TODO will make this more efficient
    for (_i, validator) in list_of_quic_address.clone().iter().enumerate() {
        for (j, _) in leaders_in_index.iter().enumerate() {
            if validator.address == leaders_in_index[j] {
                leaders_node_config_in_index[j] = validator.clone()
            }
        }
    }

    let leaders_with_quic = leaders_node_config_in_index
        .iter()
        .filter(|validator| validator.tpu_quic.is_some())
        .count();
    // let unassigned_slots = slot_leaders_ordered
    //     .iter()
    //     .filter(|&&leader_id| leader_id == u16::MAX)
    //     .count();

    // println!("{:?}", &slot_leaders_ordered);
    // println!("{:?}", &leaders_in_index);
    // println!("{:?}", &leaders_node_config_in_index);

    println!("Big Array Len {:?}", &slot_leaders_ordered.len());
    println!("Leaders Index Len {:?}", &leaders_in_index.len());
    println!("Leaders TPU Len {:?}", &leaders_node_config_in_index.len());
    println!("Scheduled leaders with TPU/QUIC addresses {leaders_with_quic}");
    // println!("Unassigned leader-schedule slots {unassigned_slots}");
    let mut leader_schedule = global_schedule.write().expect("failed to write");
    leader_schedule.leader_schedule_at_offset = slot_leaders_ordered;
    leader_schedule.current_slot = current_slot;
    leader_schedule.leader_address_at_offset_index = leaders_in_index;
    leader_schedule.leader_tpu_at_offset_index = leaders_node_config_in_index;
    leader_schedule.current_epoch = current_epoch.epoch;
    leader_schedule.epoch_start_slot = current_epoch.absolute_slot - current_epoch.slot_index;

    drop(leader_schedule);
}
