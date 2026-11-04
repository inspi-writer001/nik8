//! Opus encode/decode for voice packets: 16 kHz mono, 60 ms frames, two frames
//! (120 ms) per Opus packet.

use opus_rs::{Application, OpusDecoder, OpusEncoder};

pub const SAMPLE_RATE: i32 = 16_000;
/// 60 ms at 16 kHz, the largest single Opus frame.
pub const FRAME_SAMPLES: usize = 960;
pub const FRAMES_PER_PACKET: usize = 2;
/// 120 ms of audio per voice packet.
pub const PACKET_SAMPLES: usize = FRAME_SAMPLES * FRAMES_PER_PACKET;
pub const PACKET_MS: u64 = 120;
const BITRATE_BPS: i32 = 16_000;
const MAX_FRAME_BYTES: usize = 400;

/// Turns a stream of PCM16 samples into 120 ms Opus packets.
pub struct VoiceEncoder {
    encoder: OpusEncoder,
    pcm: Vec<i16>,
    frames: Vec<Vec<u8>>,
}

impl VoiceEncoder {
    pub fn new() -> Result<Self, String> {
        let mut encoder = OpusEncoder::new(SAMPLE_RATE, 1, Application::Voip)
            .map_err(|error| format!("opus encoder: {error}"))?;
        encoder.bitrate_bps = BITRATE_BPS;
        // Constant-size frames let two frames share one code-1 packet.
        encoder.use_cbr = true;

        Ok(Self {
            encoder,
            pcm: Vec::with_capacity(PACKET_SAMPLES),
            frames: Vec::with_capacity(FRAMES_PER_PACKET),
        })
    }

    /// Buffers `samples` and returns any completed 120 ms packets.
    pub fn push(&mut self, samples: &[i16]) -> Result<Vec<Vec<u8>>, String> {
        self.pcm.extend_from_slice(samples);
        let mut packets = Vec::new();

        while self.pcm.len() >= FRAME_SAMPLES {
            let frame: Vec<i16> = self.pcm.drain(..FRAME_SAMPLES).collect();
            self.encode_frame(&frame)?;
            if self.frames.len() == FRAMES_PER_PACKET {
                packets.extend(self.take_packets());
            }
        }
        Ok(packets)
    }

    /// Pads the remaining audio with silence and returns the final packet(s).
    pub fn flush(&mut self) -> Result<Vec<Vec<u8>>, String> {
        if !self.pcm.is_empty() {
            let mut frame = std::mem::take(&mut self.pcm);
            frame.resize(FRAME_SAMPLES, 0);
            self.encode_frame(&frame)?;
        }
        if self.frames.len() == 1 {
            self.encode_frame(&[0; FRAME_SAMPLES])?;
        }
        Ok(self.take_packets())
    }

    fn encode_frame(&mut self, pcm: &[i16]) -> Result<(), String> {
        let mut out = [0_u8; MAX_FRAME_BYTES];
        let len = self
            .encoder
            .encode_i16(pcm, FRAME_SAMPLES, &mut out)
            .map_err(|error| format!("opus encode: {error}"))?;
        self.frames.push(out[..len].to_vec());
        Ok(())
    }

    fn take_packets(&mut self) -> Vec<Vec<u8>> {
        let frames = std::mem::take(&mut self.frames);
        match frames.as_slice() {
            [] => Vec::new(),
            [first, second] => match combine_two_frames(first, second) {
                Some(packet) => vec![packet],
                None => frames,
            },
            _ => frames,
        }
    }
}

/// Splits a single-frame Opus packet into its ToC config (code bits cleared) and
/// frame payload. Handles code 0 and code 3 with one frame, which the CBR encoder
/// emits with padding to hit the exact byte budget (RFC 6716 §3.2.5).
fn single_frame(packet: &[u8]) -> Option<(u8, &[u8])> {
    let (&toc, rest) = packet.split_first()?;
    match toc & 0x03 {
        0 => Some((toc, rest)),
        3 => {
            let (&count, mut data) = rest.split_first()?;
            if count & 0x3F != 1 {
                return None;
            }
            if count & 0x40 != 0 {
                // Padding length: bytes of 255 add 254 and continue; the last adds itself.
                let mut padding = 0_usize;
                loop {
                    let (&byte, tail) = data.split_first()?;
                    data = tail;
                    if byte == 255 {
                        padding += 254;
                    } else {
                        padding += usize::from(byte);
                        break;
                    }
                }
                data = data.get(..data.len().checked_sub(padding)?)?;
            }
            Some((toc & !0x03, data))
        }
        _ => None,
    }
}

/// Combines two single-frame Opus packets with the same configuration into one
/// two-frame packet (RFC 6716 §3.2.2/3.2.3). Returns `None` if their ToC
/// configurations differ, in which case they must be sent separately.
pub fn combine_two_frames(first: &[u8], second: &[u8]) -> Option<Vec<u8>> {
    let (toc_a, payload_a) = single_frame(first)?;
    let (toc_b, payload_b) = single_frame(second)?;
    if toc_a != toc_b || payload_a.is_empty() || payload_b.is_empty() {
        return None;
    }

    let mut packet = Vec::with_capacity(3 + payload_a.len() + payload_b.len());
    if payload_a.len() == payload_b.len() {
        packet.push(toc_a | 0x01); // code 1: two equal-size frames
    } else {
        packet.push(toc_a | 0x02); // code 2: two frames, first length prefixed
        let len = payload_a.len();
        if len < 252 {
            packet.push(len as u8);
        } else {
            let first_byte = 252 + (len & 0x03);
            packet.push(first_byte as u8);
            packet.push(((len - first_byte) / 4) as u8);
        }
    }
    packet.extend_from_slice(payload_a);
    packet.extend_from_slice(payload_b);
    Some(packet)
}

/// Decodes voice packets to f32 PCM, concealing lost ones.
pub struct VoiceDecoder {
    decoder: OpusDecoder,
}

impl VoiceDecoder {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: OpusDecoder::new(SAMPLE_RATE, 1)
                .map_err(|error| format!("opus decoder: {error}"))?,
        })
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, String> {
        let mut pcm = vec![0.0_f32; PACKET_SAMPLES];
        let samples = self
            .decoder
            .decode(packet, PACKET_SAMPLES, &mut pcm)
            .map_err(|error| format!("opus decode: {error}"))?;
        pcm.truncate(samples);
        Ok(pcm)
    }

    /// Packet loss concealment for one missing 120 ms packet.
    pub fn conceal(&mut self) -> Vec<f32> {
        self.decode(&[]).unwrap_or_else(|_| vec![0.0; PACKET_SAMPLES])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(samples: usize) -> Vec<i16> {
        (0..samples)
            .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 8_000.0) as i16)
            .collect()
    }

    #[test]
    fn encodes_120ms_packets_that_decode_to_120ms() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();

        // 100 ms chunks, like a mic callback that doesn't align with frames.
        let mut packets = Vec::new();
        for chunk in tone(PACKET_SAMPLES * 3).chunks(1_600) {
            packets.extend(encoder.push(chunk).unwrap());
        }
        assert_eq!(packets.len(), 3);

        for packet in &packets {
            // One packet per 120 ms, sized for a 16 kbps budget.
            assert!(packet.len() <= 260, "packet is {} bytes", packet.len());
            assert_eq!(decoder.decode(packet).unwrap().len(), PACKET_SAMPLES);
        }
    }

    #[test]
    fn flush_pads_a_partial_packet() {
        let mut encoder = VoiceEncoder::new().unwrap();
        assert!(encoder.push(&tone(500)).unwrap().is_empty());

        let packets = encoder.flush().unwrap();
        assert_eq!(packets.len(), 1);
        let mut decoder = VoiceDecoder::new().unwrap();
        assert_eq!(decoder.decode(&packets[0]).unwrap().len(), PACKET_SAMPLES);
        assert!(encoder.flush().unwrap().is_empty());
    }

    #[test]
    fn conceals_lost_packets() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();
        let packet = encoder.push(&tone(PACKET_SAMPLES)).unwrap().remove(0);
        decoder.decode(&packet).unwrap();
        assert_eq!(decoder.conceal().len(), PACKET_SAMPLES);
    }

    #[test]
    fn combines_frames_with_rfc_6716_framing() {
        assert_eq!(
            combine_two_frames(&[0x48, 1, 2], &[0x48, 3, 4]),
            Some(vec![0x49, 1, 2, 3, 4])
        );
        assert_eq!(
            combine_two_frames(&[0x48, 1], &[0x48, 3, 4]),
            Some(vec![0x4A, 1, 1, 3, 4])
        );
        let long = vec![0x48; 301];
        let combined = combine_two_frames(&long, &[0x48, 9]).unwrap();
        // 300 = 252 + 0 + 4 * 12
        assert_eq!(&combined[..3], &[0x4A, 252, 12]);
        assert_eq!(combine_two_frames(&[0x48, 1], &[0x50, 1]), None);
    }

    #[test]
    fn unwraps_padded_code_3_frames_before_combining() {
        // Code 3, one frame, padding flag, 2 bytes of padding.
        let padded = [0x4B, 0x41, 2, 7, 8, 0, 0];
        assert_eq!(single_frame(&padded), Some((0x48, &[7, 8][..])));
        assert_eq!(
            combine_two_frames(&[0x48, 1, 2], &padded),
            Some(vec![0x49, 1, 2, 7, 8])
        );
        // 255 continues: 254 + 1 = 255 bytes of padding.
        let mut long_pad = vec![0x4B, 0x41, 255, 1, 9];
        long_pad.extend(std::iter::repeat_n(0, 255));
        assert_eq!(single_frame(&long_pad), Some((0x48, &[9][..])));
        // More than one frame isn't a single frame.
        assert_eq!(single_frame(&[0x4B, 0x02, 1, 2]), None);
    }
}

