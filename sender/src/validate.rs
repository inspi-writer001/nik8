use std::{env, str::FromStr};

use solana_address::Address;
use solana_transaction::Transaction;

use crate::Nik8Error;

/// The deployed Nik8 program. Override with `NIK8_PROGRAM_ID` for other deployments.
const DEFAULT_PROGRAM_ID: &str = "NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU";

pub fn program_id() -> Result<Address, Nik8Error> {
    let id = env::var("NIK8_PROGRAM_ID").unwrap_or_else(|_| DEFAULT_PROGRAM_ID.to_owned());
    Address::from_str(&id).map_err(|_| Nik8Error::InvalidTransaction(format!("bad NIK8_PROGRAM_ID {id}")))
}

/// The ingress only relays Nik8 voice transactions, so it can't be used as an open
/// relay: a legacy transaction whose signatures verify and whose every instruction
/// calls `program_id`. Returns the fee payer (the sender's session key).
pub fn validate_nik8_transaction(wire: &[u8], program_id: &Address) -> Result<Address, Nik8Error> {
    let invalid = |reason: &str| Nik8Error::InvalidTransaction(reason.to_owned());

    let transaction: Transaction =
        bincode::deserialize(wire).map_err(|_| invalid("not a legacy transaction"))?;
    let message = &transaction.message;

    if message.instructions.is_empty() {
        return Err(invalid("no instructions"));
    }
    let only_nik8 = message.instructions.iter().all(|instruction| {
        message
            .account_keys
            .get(usize::from(instruction.program_id_index))
            .is_some_and(|program| program == program_id)
    });
    if !only_nik8 {
        return Err(invalid("instruction for another program"));
    }
    transaction
        .verify()
        .map_err(|_| invalid("signature verification failed"))?;

    message
        .account_keys
        .first()
        .copied()
        .ok_or_else(|| invalid("no fee payer"))
}

#[cfg(test)]
mod tests {
    use solana_hash::Hash;
    use solana_instruction::{AccountMeta, Instruction};
    use solana_keypair::Keypair;
    use solana_signer::Signer;

    use super::*;

    fn program() -> Address {
        Address::from_str(DEFAULT_PROGRAM_ID).unwrap()
    }

    fn signed(keypair: &Keypair, program_ids: &[Address]) -> Vec<u8> {
        let instructions: Vec<Instruction> = program_ids
            .iter()
            .map(|program_id| Instruction {
                program_id: *program_id,
                accounts: vec![AccountMeta::new(keypair.pubkey(), true)],
                data: vec![1, 2, 3],
            })
            .collect();
        let transaction = Transaction::new_signed_with_payer(
            &instructions,
            Some(&keypair.pubkey()),
            &[keypair],
            Hash::new_from_array([7; 32]),
        );
        bincode::serialize(&transaction).unwrap()
    }

    #[test]
    fn accepts_signed_nik8_transactions() {
        let keypair = Keypair::new();
        let wire = signed(&keypair, &[program(), program()]);
        assert_eq!(validate_nik8_transaction(&wire, &program()).unwrap(), keypair.pubkey());
    }

    #[test]
    fn rejects_other_programs() {
        let keypair = Keypair::new();
        let wire = signed(&keypair, &[program(), Address::new_from_array([0; 32])]);
        assert!(validate_nik8_transaction(&wire, &program()).is_err());
    }

    #[test]
    fn rejects_tampered_or_garbage_bytes() {
        let keypair = Keypair::new();
        let mut wire = signed(&keypair, &[program()]);
        *wire.last_mut().unwrap() ^= 0xFF; // flip a data byte after signing
        assert!(validate_nik8_transaction(&wire, &program()).is_err());
        assert!(validate_nik8_transaction(&[1, 2, 3], &program()).is_err());
    }
}
