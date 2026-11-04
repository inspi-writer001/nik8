use std::{error::Error, fmt};

use nik8_shared::{IX_ONEWAY_MESSAGE_DISCRIMINATOR, VoicePacket, VoicePacketError};
use yellowstone_grpc_proto::prelude::{SubscribeUpdate, subscribe_update::UpdateOneof};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nik8InstructionError {
    MissingTransaction,
    MissingMessage,
    ProgramIdIndexOutOfBounds(u32),
    InvalidVoicePacket(VoicePacketError),
}

impl Nik8InstructionError {
    /// A well-formed packet in a version or codec this build doesn't understand.
    /// Rising counts mean receivers need upgrading, not that the data is bad.
    pub fn is_unsupported_format(&self) -> bool {
        matches!(
            self,
            Self::InvalidVoicePacket(VoicePacketError::UnsupportedFormat)
        )
    }
}

impl fmt::Display for Nik8InstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTransaction => {
                formatter.write_str("transaction update has no transaction")
            }
            Self::MissingMessage => formatter.write_str("transaction has no message"),
            Self::ProgramIdIndexOutOfBounds(index) => {
                write!(
                    formatter,
                    "program ID index {index} is outside the account keys"
                )
            }
            Self::InvalidVoicePacket(error) => {
                write!(formatter, "invalid Nik8 voice packet: {error:?}")
            }
        }
    }
}

impl Error for Nik8InstructionError {}

/// Voice packets found in one update, plus anything that was skipped.
///
/// A bad instruction never hides valid ones in the same transaction, and a bad
/// update never ends the stream; callers count `rejected` and carry on.
#[derive(Default)]
pub struct Extracted<'a> {
    pub packets: Vec<VoicePacket<'a>>,
    pub rejected: Vec<Nik8InstructionError>,
}

impl Extracted<'_> {
    fn rejected(error: Nik8InstructionError) -> Self {
        Self {
            packets: Vec::new(),
            rejected: vec![error],
        }
    }
}

/// Extract Nik8 one-way voice packets for `program_id` from one Yellowstone update.
pub fn extract_voice_packets<'a>(
    update: &'a SubscribeUpdate,
    program_id: &[u8; 32],
) -> Extracted<'a> {
    let Some(UpdateOneof::Transaction(transaction_update)) = &update.update_oneof else {
        return Extracted::default();
    };

    let Some(transaction_info) = transaction_update.transaction.as_ref() else {
        return Extracted::rejected(Nik8InstructionError::MissingTransaction);
    };

    // ignore failed transaction
    if transaction_info
        .meta
        .as_ref()
        .is_some_and(|meta| meta.err.is_some())
    {
        return Extracted::default();
    }

    let Some(transaction) = transaction_info.transaction.as_ref() else {
        return Extracted::rejected(Nik8InstructionError::MissingTransaction);
    };
    let Some(message) = transaction.message.as_ref() else {
        return Extracted::rejected(Nik8InstructionError::MissingMessage);
    };

    let mut extracted = Extracted::default();
    for instruction in &message.instructions {
        // Program IDs are always static account keys, even in v0 transactions.
        let Some(instruction_program_id) = message
            .account_keys
            .get(instruction.program_id_index as usize)
        else {
            extracted
                .rejected
                .push(Nik8InstructionError::ProgramIdIndexOutOfBounds(
                    instruction.program_id_index,
                ));
            continue;
        };

        if instruction_program_id.as_slice() != program_id {
            continue;
        }

        let Some((&discriminator, payload)) = instruction.data.split_first() else {
            continue;
        };
        if discriminator != IX_ONEWAY_MESSAGE_DISCRIMINATOR {
            continue;
        }

        match VoicePacket::parse(payload) {
            Ok(packet) => extracted.packets.push(packet),
            Err(error) => extracted
                .rejected
                .push(Nik8InstructionError::InvalidVoicePacket(error)),
        }
    }

    extracted
}

#[cfg(test)]
mod tests {
    use super::*;
    use yellowstone_grpc_proto::prelude::{
        CompiledInstruction, Message, SubscribeUpdatePing, SubscribeUpdateTransaction,
        SubscribeUpdateTransactionInfo, Transaction,
    };

    const PROGRAM: [u8; 32] = [9; 32];
    const OTHER_PROGRAM: [u8; 32] = [3; 32];

    fn voice_payload(version: u8, sequence: u32) -> Vec<u8> {
        let mut data = vec![IX_ONEWAY_MESSAGE_DISCRIMINATOR, version, 1];
        data.extend_from_slice(&[7; 16]);
        data.extend_from_slice(&[8; 32]);
        data.extend_from_slice(&sequence.to_le_bytes());
        data.extend_from_slice(&0_u64.to_le_bytes());
        data.extend_from_slice(&[0xF8, 0xFF, 0xFE]);
        data
    }

    fn instruction(program_id_index: u32, data: Vec<u8>) -> CompiledInstruction {
        CompiledInstruction {
            program_id_index,
            accounts: Vec::new(),
            data,
        }
    }

    fn update(message: Option<Message>) -> SubscribeUpdate {
        SubscribeUpdate {
            update_oneof: Some(UpdateOneof::Transaction(SubscribeUpdateTransaction {
                transaction: Some(SubscribeUpdateTransactionInfo {
                    transaction: Some(Transaction {
                        message,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn message(instructions: Vec<CompiledInstruction>) -> Message {
        Message {
            account_keys: vec![PROGRAM.to_vec(), OTHER_PROGRAM.to_vec()],
            instructions,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_valid_packets_next_to_invalid_ones() {
        let update = update(Some(message(vec![
            instruction(0, voice_payload(1, 1)),
            instruction(0, vec![IX_ONEWAY_MESSAGE_DISCRIMINATOR, 1, 1]),
            instruction(0, voice_payload(1, 2)),
        ])));

        let extracted = extract_voice_packets(&update, &PROGRAM);
        let sequences: Vec<u32> = extracted
            .packets
            .iter()
            .map(|packet| packet.header.sequence())
            .collect();
        assert_eq!(sequences, [1, 2]);
        assert_eq!(extracted.rejected.len(), 1);
    }

    #[test]
    fn reports_unknown_versions_as_unsupported() {
        let update = update(Some(message(vec![instruction(0, voice_payload(2, 1))])));

        let extracted = extract_voice_packets(&update, &PROGRAM);
        assert!(extracted.packets.is_empty());
        assert!(extracted.rejected[0].is_unsupported_format());
    }

    #[test]
    fn skips_out_of_bounds_program_index_and_keeps_going() {
        let update = update(Some(message(vec![
            instruction(7, voice_payload(1, 1)),
            instruction(0, voice_payload(1, 2)),
        ])));

        let extracted = extract_voice_packets(&update, &PROGRAM);
        assert_eq!(extracted.packets.len(), 1);
        assert_eq!(
            extracted.rejected,
            [Nik8InstructionError::ProgramIdIndexOutOfBounds(7)]
        );
    }

    #[test]
    fn ignores_other_programs_and_discriminators() {
        let mut other_discriminator = voice_payload(1, 1);
        other_discriminator[0] = 2;
        let update = update(Some(message(vec![
            instruction(1, voice_payload(1, 1)),
            instruction(0, other_discriminator),
            instruction(0, Vec::new()),
        ])));

        let extracted = extract_voice_packets(&update, &PROGRAM);
        assert!(extracted.packets.is_empty());
        assert!(extracted.rejected.is_empty());
    }

    #[test]
    fn malformed_updates_are_rejected_not_fatal() {
        let update = update(None);
        let extracted = extract_voice_packets(&update, &PROGRAM);
        assert_eq!(extracted.rejected, [Nik8InstructionError::MissingMessage]);
    }

    #[test]
    fn non_transaction_updates_are_ignored() {
        let ping = SubscribeUpdate {
            update_oneof: Some(UpdateOneof::Ping(SubscribeUpdatePing {})),
            ..Default::default()
        };

        let extracted = extract_voice_packets(&ping, &PROGRAM);
        assert!(extracted.packets.is_empty());
        assert!(extracted.rejected.is_empty());
    }
}
