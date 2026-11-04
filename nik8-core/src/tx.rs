//! Minimal legacy transaction building for the two Nik8 instructions the app sends.
//! Hand-rolled to keep the mobile library small; tests compare the bytes against
//! `solana-transaction`.

use ed25519_dalek::{Signer, SigningKey};
use nik8_shared::{VoicePacket, VoicePacketError};
use solana_address::Address;

/// Solana's maximum serialized transaction size.
pub const MAX_TRANSACTION_LEN: usize = 1_232;
pub const SYSTEM_PROGRAM_ID: [u8; 32] = [0; 32];
/// `Nik8Ix::Cre8Account`.
pub const CREATE_SENDER_DISCRIMINATOR: u8 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TxError {
    Packet(VoicePacketError),
    TooLarge(usize),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Packet(error) => write!(formatter, "invalid voice packet: {error:?}"),
            Self::TooLarge(len) => write!(
                formatter,
                "transaction is {len} bytes, over the {MAX_TRANSACTION_LEN}-byte limit"
            ),
        }
    }
}

impl std::error::Error for TxError {}

/// The device's session keypair. It is the `sender` on every voice packet.
pub struct SessionKey(SigningKey);

impl SessionKey {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    pub fn pubkey(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

/// `[b"nik8_config"]` PDA holding the global packet fee.
pub fn global_config_address(program_id: &[u8; 32]) -> [u8; 32] {
    find_pda(&[b"nik8_config"], program_id)
}

/// `[b"nik8_user", sender]` PDA; the sender's fee vault, created by `Cre8Account`.
pub fn sender_vault_address(program_id: &[u8; 32], sender: &[u8; 32]) -> [u8; 32] {
    find_pda(&[b"nik8_user", sender], program_id)
}

fn find_pda(seeds: &[&[u8]], program_id: &[u8; 32]) -> [u8; 32] {
    Address::find_program_address(seeds, &Address::new_from_array(*program_id))
        .0
        .to_bytes()
}

/// `Cre8Account`: registers `key` as a sender by creating its fee vault PDA.
/// `key` signs and pays rent.
pub fn build_create_sender_tx(
    key: &SessionKey,
    program_id: &[u8; 32],
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, TxError> {
    let sender = key.pubkey();
    let accounts = [
        AccountMeta::signer(sender),
        AccountMeta::writable(sender_vault_address(program_id, &sender)),
        AccountMeta::readonly(SYSTEM_PROGRAM_ID),
    ];
    sign_instructions(
        key,
        program_id,
        &accounts,
        &[vec![CREATE_SENDER_DISCRIMINATOR]],
        recent_blockhash,
    )
}

/// One `OnewayMessage` instruction per packet, all from `key`, in one transaction.
pub fn build_voice_tx(
    key: &SessionKey,
    program_id: &[u8; 32],
    packets: &[VoicePacket<'_>],
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, TxError> {
    let instructions = packets
        .iter()
        .map(|packet| {
            let mut data = vec![0_u8; packet.instruction_len()];
            packet.encode_instruction(&mut data).map_err(TxError::Packet)?;
            Ok(data)
        })
        .collect::<Result<Vec<_>, TxError>>()?;

    let sender = key.pubkey();
    let accounts = [
        AccountMeta::signer(sender),
        AccountMeta::writable(sender_vault_address(program_id, &sender)),
        AccountMeta::readonly(global_config_address(program_id)),
        AccountMeta::readonly(SYSTEM_PROGRAM_ID),
    ];
    sign_instructions(key, program_id, &accounts, &instructions, recent_blockhash)
}

/// The first signature, which is the transaction ID.
pub fn transaction_signature(transaction: &[u8]) -> Option<[u8; 64]> {
    transaction.get(1..65)?.try_into().ok()
}

#[derive(Clone, Copy)]
struct AccountMeta {
    key: [u8; 32],
    signer: bool,
    writable: bool,
}

impl AccountMeta {
    fn signer(key: [u8; 32]) -> Self {
        Self {
            key,
            signer: true,
            writable: true,
        }
    }

    fn writable(key: [u8; 32]) -> Self {
        Self {
            key,
            signer: false,
            writable: true,
        }
    }

    fn readonly(key: [u8; 32]) -> Self {
        Self {
            key,
            signer: false,
            writable: false,
        }
    }
}

/// Compiles and signs a legacy message whose instructions all call `program_id`
/// with the same `accounts`. The only signer must be `accounts[0]`, the fee payer.
fn sign_instructions(
    key: &SessionKey,
    program_id: &[u8; 32],
    accounts: &[AccountMeta],
    instructions: &[Vec<u8>],
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, TxError> {
    debug_assert!(accounts[0].signer && accounts[0].key == key.pubkey());
    debug_assert!(accounts[1..].iter().all(|account| !account.signer));

    // Account order: payer, writable non-signers, readonly non-signers (incl. the
    // program ID). Sorted by address within each group, as `solana-message` does.
    let mut writable: Vec<[u8; 32]> = accounts[1..]
        .iter()
        .filter(|a| a.writable)
        .map(|a| a.key)
        .collect();
    let mut readonly: Vec<[u8; 32]> = accounts[1..]
        .iter()
        .filter(|a| !a.writable)
        .map(|a| a.key)
        .chain([*program_id])
        .collect();
    writable.sort_unstable();
    readonly.sort_unstable();
    let readonly_unsigned = readonly.len();
    let keys: Vec<[u8; 32]> = [accounts[0].key]
        .into_iter()
        .chain(writable)
        .chain(readonly)
        .collect();

    let index = |target: &[u8; 32]| keys.iter().position(|k| k == target).unwrap() as u8;

    let mut message = Vec::with_capacity(MAX_TRANSACTION_LEN);
    message.extend_from_slice(&[1, 0, readonly_unsigned as u8]);
    write_compact_u16(&mut message, keys.len());
    for key in &keys {
        message.extend_from_slice(key);
    }
    message.extend_from_slice(recent_blockhash);
    write_compact_u16(&mut message, instructions.len());
    for data in instructions {
        message.push(index(program_id));
        write_compact_u16(&mut message, accounts.len());
        message.extend(accounts.iter().map(|account| index(&account.key)));
        write_compact_u16(&mut message, data.len());
        message.extend_from_slice(data);
    }

    let mut transaction = Vec::with_capacity(1 + 64 + message.len());
    write_compact_u16(&mut transaction, 1);
    transaction.extend_from_slice(&key.sign(&message));
    transaction.extend_from_slice(&message);

    if transaction.len() > MAX_TRANSACTION_LEN {
        return Err(TxError::TooLarge(transaction.len()));
    }
    Ok(transaction)
}

/// Solana's "shortvec" length prefix.
fn write_compact_u16(out: &mut Vec<u8>, value: usize) {
    let mut value = value as u16;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nik8_consumer::NIK8_PROGRAM_ID_BYTES;
    use nik8_shared::VoicePacketHeader;
    use solana_hash::Hash;
    use solana_instruction::{AccountMeta as SolMeta, Instruction};
    use solana_keypair::Keypair;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer as _;
    use solana_transaction::Transaction;

    const SEED: [u8; 32] = [42; 32];
    const BLOCKHASH: [u8; 32] = [7; 32];

    fn reference(instructions: &[Instruction]) -> Vec<u8> {
        let keypair = Keypair::new_from_array(SEED);
        let transaction = Transaction::new_signed_with_payer(
            instructions,
            Some(&keypair.pubkey()),
            &[&keypair],
            Hash::new_from_array(BLOCKHASH),
        );
        bincode::serialize(&transaction).unwrap()
    }

    fn pubkey(bytes: [u8; 32]) -> Pubkey {
        Pubkey::new_from_array(bytes)
    }

    #[test]
    fn session_key_matches_solana_keypair() {
        let keypair = Keypair::new_from_array(SEED);
        assert_eq!(
            SessionKey::from_seed(&SEED).pubkey(),
            keypair.pubkey().to_bytes()
        );
    }

    #[test]
    fn create_sender_tx_matches_solana_transaction() {
        let key = SessionKey::from_seed(&SEED);
        let sender = key.pubkey();
        let program = NIK8_PROGRAM_ID_BYTES;

        let expected = reference(&[Instruction {
            program_id: pubkey(program),
            accounts: vec![
                SolMeta::new(pubkey(sender), true),
                SolMeta::new(pubkey(sender_vault_address(&program, &sender)), false),
                SolMeta::new_readonly(pubkey(SYSTEM_PROGRAM_ID), false),
            ],
            data: vec![CREATE_SENDER_DISCRIMINATOR],
        }]);

        assert_eq!(
            build_create_sender_tx(&key, &program, &BLOCKHASH).unwrap(),
            expected
        );
    }

    fn voice_reference(packets: &[VoicePacket<'_>]) -> Vec<u8> {
        let sender = SessionKey::from_seed(&SEED).pubkey();
        let program = NIK8_PROGRAM_ID_BYTES;
        let instructions: Vec<Instruction> = packets
            .iter()
            .map(|packet| {
                let mut data = vec![0; packet.instruction_len()];
                packet.encode_instruction(&mut data).unwrap();
                Instruction {
                    program_id: pubkey(program),
                    accounts: vec![
                        SolMeta::new(pubkey(sender), true),
                        SolMeta::new(pubkey(sender_vault_address(&program, &sender)), false),
                        SolMeta::new_readonly(pubkey(global_config_address(&program)), false),
                        SolMeta::new_readonly(pubkey(SYSTEM_PROGRAM_ID), false),
                    ],
                    data,
                }
            })
            .collect();
        reference(&instructions)
    }

    #[test]
    fn voice_tx_matches_solana_transaction() {
        let key = SessionKey::from_seed(&SEED);
        let opus = [0xF8_u8; 240];
        let packet = VoicePacket {
            header: VoicePacketHeader::new([1; 16], [2; 32], 3, 4),
            opus_packet: &opus,
        };

        let built = build_voice_tx(&key, &NIK8_PROGRAM_ID_BYTES, &[packet], &BLOCKHASH).unwrap();
        let packet = VoicePacket {
            header: VoicePacketHeader::new([1; 16], [2; 32], 3, 4),
            opus_packet: &opus,
        };
        assert_eq!(built, voice_reference(&[packet]));
        // 120 ms at 16 kbps is ~240 bytes of Opus; the plan budgets ~573 bytes.
        assert_eq!(built.len(), 573);
    }

    #[test]
    fn two_packet_voice_tx_matches_solana_transaction() {
        let key = SessionKey::from_seed(&SEED);
        let opus = [0xF8_u8; 240];
        let packets = |offset: u32| {
            [0, 1].map(|i| VoicePacket {
                header: VoicePacketHeader::new([1; 16], [2; 32], offset + i, 4),
                opus_packet: &opus,
            })
        };

        let built = build_voice_tx(&key, &NIK8_PROGRAM_ID_BYTES, &packets(10), &BLOCKHASH).unwrap();
        assert_eq!(built, voice_reference(&packets(10)));
        // Second instruction adds ~311 bytes; 240 ms of audio still fits easily.
        assert_eq!(built.len(), 884);
    }

    #[test]
    fn oversized_voice_tx_is_rejected() {
        let key = SessionKey::from_seed(&SEED);
        let opus = [0_u8; 1_000];
        let packet = VoicePacket {
            header: VoicePacketHeader::new([1; 16], [2; 32], 3, 4),
            opus_packet: &opus,
        };

        assert!(matches!(
            build_voice_tx(&key, &NIK8_PROGRAM_ID_BYTES, &[packet], &BLOCKHASH),
            Err(TxError::TooLarge(_))
        ));
    }

    #[test]
    fn compact_u16_encoding() {
        for (value, expected) in [
            (0, vec![0x00]),
            (0x7f, vec![0x7f]),
            (0x80, vec![0x80, 0x01]),
            (0x3fff, vec![0xff, 0x7f]),
            (0x4000, vec![0x80, 0x80, 0x01]),
        ] {
            let mut out = Vec::new();
            write_compact_u16(&mut out, value);
            assert_eq!(out, expected, "value {value:#x}");
        }
    }
}
