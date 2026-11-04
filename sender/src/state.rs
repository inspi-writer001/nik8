use core::net::SocketAddr;
use std::io::Error;
use std::sync::LazyLock;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct MyLeaderSchedule {
    // pub validators: Box<HashMap<String, Validator>>,
    /// when we get a Hashmap from leader schedule, we wanna extract the validators addresses and replace them with an identifier 0, 1, 2, 3, ... - that way we can easily say `[34, 34, 34, 34, 1, 1, 1, 1 ...]` means validator id 34 is a `Leader` for the first, second, third, fourth slots in the epoch and validator id 1 is `Leader` for fifth, sixth, seventh, and eighth slots in the epoch, and so on..
    pub leader_schedule_at_offset: Vec<u16>,
    /// A long list of all the Leaders in the schedule as they appear in `keys()` of the Hashmap from `get_leaders_schedule`
    pub leader_address_at_offset_index: Vec<String>,
    pub current_slot: u64,
    pub current_epoch: u64,
    /// what global slot number did epoch `e` start at?
    pub epoch_start_slot: u64,
    pub leader_tpu_at_offset_index: Vec<Validator>,
}

impl MyLeaderSchedule {
    pub const SLOT_PER_EPOCH: usize = 432_000;
    pub fn new() -> Self {
        Self {
            leader_schedule_at_offset: {
                let leaders_upcoming = Vec::<u16>::with_capacity(Self::SLOT_PER_EPOCH);
                leaders_upcoming
            },
            current_epoch: 0u64,
            current_slot: 0u64,
            epoch_start_slot: 0u64,
            leader_tpu_at_offset_index: Vec::new(),
            leader_address_at_offset_index: Vec::new(), // number of leaders in an epoch may change based on players participating in the network
                                                        // validators: {
                                                        //     let leaders_in = HashMap::new();
                                                        //     Box::new(leaders_in)
                                                        // },
        }
    }

    pub fn set_coming_validators(&mut self, validators: Vec<u16>) -> Result<(), Error> {
        self.leader_schedule_at_offset.clone_from_slice(&validators);
        Ok(())
    }

    // pub fn current_and_upcoming_leaders(&self) -> Result<&[Address], Error> {
    //     let slots_have_passed_since_epoch = (self.current_slot - self.epoch_start_slot) as usize;
    //     let leaders_in_range = (slots_have_passed_since_epoch + 6) as usize;

    //     let some_validators =
    //         &self.leader_schedule_at_offset[slots_have_passed_since_epoch..leaders_in_range];
    //     Ok(some_validators)
    // }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Validator {
    pub address: String,
    pub tpu_quic: Option<SocketAddr>,
    pub rpc: Option<SocketAddr>,
    pub tpu_udp: Option<SocketAddr>,
}

// impl Default for Validator {
//     fn default() -> Self {
//         Self {
//             address: "".to_string(),
//             tpu_quic: None,
//             rpc: None,
//             tpu_udp: None,
//         }
//     }
// }

pub static MY_LEADER_SCHEDULE: LazyLock<RwLock<MyLeaderSchedule>> =
    LazyLock::new(|| RwLock::new(MyLeaderSchedule::new()));
