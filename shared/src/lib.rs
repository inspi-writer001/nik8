#![no_std]

use bytemuck::{CheckedBitPattern, NoUninit};

/// Nik8 instruction discriminator for a one-way voice packet.
pub const IX_ONEWAY_MESSAGE_DISCRIMINATOR: u8 = 1;
/// The only currently supported packet format.
pub const VOICE_PACKET_VERSION: u8 = 1;
/// RFC 6716 Opus packet codec identifier in the Nik8 protocol.
pub const CODEC_OPUS: u8 = 1;
/// RFC 6716 caps an Opus packet at 1,275 bytes.
pub const MAX_OPUS_PACKET_LEN: usize = 1_275;

/// The fixed, byte-aligned part of a Nik8 one-way voice packet.
///
/// The complete instruction data is `IX_ONEWAY_MESSAGE || VoicePacketHeader || opus`.
#[repr(C)]
#[derive(NoUninit, CheckedBitPattern, Clone, Copy, Debug, Eq, PartialEq)]
pub struct VoicePacketHeader {
    pub version: u8,
    pub codec: u8,
    pub session_id: [u8; 16],
    /// The 32-byte Solana address the packet is intended for.
    pub recipient: [u8; 32],
    pub sequence_le: [u8; 4],
    pub capture_timestamp_ms_le: [u8; 8],
}

impl VoicePacketHeader {
    pub const WIRE_LEN: usize = core::mem::size_of::<Self>();

    /// A v1 Opus header.
    pub const fn new(
        session_id: [u8; 16],
        recipient: [u8; 32],
        sequence: u32,
        capture_timestamp_ms: u64,
    ) -> Self {
        Self {
            version: VOICE_PACKET_VERSION,
            codec: CODEC_OPUS,
            session_id,
            recipient,
            sequence_le: sequence.to_le_bytes(),
            capture_timestamp_ms_le: capture_timestamp_ms.to_le_bytes(),
        }
    }

    pub const fn sequence(&self) -> u32 {
        u32::from_le_bytes(self.sequence_le)
    }

    pub const fn capture_timestamp_ms(&self) -> u64 {
        u64::from_le_bytes(self.capture_timestamp_ms_le)
    }
}

/// A validated, borrowed payload excluding its instruction discriminator.
pub struct VoicePacket<'a> {
    pub header: VoicePacketHeader,
    pub opus_packet: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoicePacketError {
    MissingOpusPacket,
    UnsupportedFormat,
    OpusPacketTooLarge,
    /// The output buffer passed to `encode_instruction` is too small.
    BufferTooSmall,
}

impl<'a> VoicePacket<'a> {
    pub const VERSION: u8 = VOICE_PACKET_VERSION;
    pub const CODEC_OPUS: u8 = CODEC_OPUS;
    pub const MAX_OPUS_PACKET_LEN: usize = MAX_OPUS_PACKET_LEN;

    pub fn parse(data: &'a [u8]) -> Result<Self, VoicePacketError> {
        if data.len() <= VoicePacketHeader::WIRE_LEN {
            return Err(VoicePacketError::MissingOpusPacket);
        }

        let header = *bytemuck::checked::try_from_bytes::<VoicePacketHeader>(
            &data[..VoicePacketHeader::WIRE_LEN],
        )
        .map_err(|_| VoicePacketError::UnsupportedFormat)?;

        if header.version != VOICE_PACKET_VERSION || header.codec != CODEC_OPUS {
            return Err(VoicePacketError::UnsupportedFormat);
        }

        let opus_packet = &data[VoicePacketHeader::WIRE_LEN..];
        if opus_packet.len() > MAX_OPUS_PACKET_LEN {
            return Err(VoicePacketError::OpusPacketTooLarge);
        }

        Ok(Self {
            header,
            opus_packet,
        })
    }

    /// Length of the complete `OnewayMessage` instruction data.
    pub fn instruction_len(&self) -> usize {
        1 + VoicePacketHeader::WIRE_LEN + self.opus_packet.len()
    }

    /// Writes `IX_ONEWAY_MESSAGE || header || opus` into `out` and returns the
    /// number of bytes written. Enforces the same limits as `parse`.
    pub fn encode_instruction(&self, out: &mut [u8]) -> Result<usize, VoicePacketError> {
        if self.opus_packet.is_empty() {
            return Err(VoicePacketError::MissingOpusPacket);
        }
        if self.opus_packet.len() > MAX_OPUS_PACKET_LEN {
            return Err(VoicePacketError::OpusPacketTooLarge);
        }
        let len = self.instruction_len();
        let Some(out) = out.get_mut(..len) else {
            return Err(VoicePacketError::BufferTooSmall);
        };

        let (discriminator, rest) = out.split_at_mut(1);
        let (header, opus) = rest.split_at_mut(VoicePacketHeader::WIRE_LEN);
        discriminator[0] = IX_ONEWAY_MESSAGE_DISCRIMINATOR;
        header.copy_from_slice(bytemuck::bytes_of(&self.header));
        opus.copy_from_slice(self.opus_packet);
        Ok(len)
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::{
        IX_ONEWAY_MESSAGE_DISCRIMINATOR, VoicePacket, VoicePacketError, VoicePacketHeader,
    };

    #[test]
    fn parses_a_version_one_opus_packet() {
        let mut data = std::vec![VoicePacket::VERSION, VoicePacket::CODEC_OPUS];
        data.extend_from_slice(&[7; 16]);
        data.extend_from_slice(&[8; 32]);
        data.extend_from_slice(&42_u32.to_le_bytes());
        data.extend_from_slice(&1_700_000_000_000_u64.to_le_bytes());
        data.extend_from_slice(&[0xF8, 0xFF, 0xFE]);

        let packet = VoicePacket::parse(&data).unwrap();
        assert_eq!(packet.header.session_id, [7; 16]);
        assert_eq!(packet.header.recipient, [8; 32]);
        assert_eq!(packet.header.sequence(), 42);
        assert_eq!(packet.header.capture_timestamp_ms(), 1_700_000_000_000);
        assert_eq!(packet.opus_packet, [0xF8, 0xFF, 0xFE]);
    }

    #[test]
    fn rejects_missing_or_oversized_opus_data() {
        assert_eq!(VoicePacketHeader::WIRE_LEN, 62);
        assert!(VoicePacket::parse(&[VoicePacket::VERSION; VoicePacketHeader::WIRE_LEN]).is_err());

        let mut data = std::vec![VoicePacket::VERSION, VoicePacket::CODEC_OPUS];
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&[0; 32]);
        data.extend_from_slice(&0_u32.to_le_bytes());
        data.extend_from_slice(&0_u64.to_le_bytes());
        data.extend_from_slice(&std::vec![0; VoicePacket::MAX_OPUS_PACKET_LEN + 1]);
        assert!(VoicePacket::parse(&data).is_err());
    }

    #[test]
    fn encoded_instruction_round_trips_through_parse() {
        let packet = VoicePacket {
            header: VoicePacketHeader::new([7; 16], [8; 32], 42, 1_700_000_000_000),
            opus_packet: &[0xF8, 0xFF, 0xFE],
        };
        let mut out = [0_u8; 128];
        let len = packet.encode_instruction(&mut out).unwrap();

        assert_eq!(len, 1 + VoicePacketHeader::WIRE_LEN + 3);
        assert_eq!(out[0], IX_ONEWAY_MESSAGE_DISCRIMINATOR);
        let parsed = VoicePacket::parse(&out[1..len]).unwrap();
        assert_eq!(parsed.header, packet.header);
        assert_eq!(parsed.opus_packet, packet.opus_packet);
    }

    #[test]
    fn encode_enforces_limits() {
        let header = VoicePacketHeader::new([0; 16], [0; 32], 0, 0);
        let mut out = [0_u8; 2_048];

        let empty = VoicePacket {
            header,
            opus_packet: &[],
        };
        assert_eq!(
            empty.encode_instruction(&mut out),
            Err(VoicePacketError::MissingOpusPacket)
        );

        let big = [0_u8; VoicePacket::MAX_OPUS_PACKET_LEN + 1];
        let too_big = VoicePacket {
            header,
            opus_packet: &big,
        };
        assert_eq!(
            too_big.encode_instruction(&mut out),
            Err(VoicePacketError::OpusPacketTooLarge)
        );

        let small = VoicePacket {
            header,
            opus_packet: &[1],
        };
        assert_eq!(
            small.encode_instruction(&mut out[..10]),
            Err(VoicePacketError::BufferTooSmall)
        );
    }
}
