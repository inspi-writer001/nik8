use pinocchio::{AccountView, ProgramResult, error::ProgramError};
use pinocchio_system::instructions::Transfer;

use crate::{
    errors::Nik8ProgramError,
    state::{GlobalConfig, SenderAccount, VoicePacket},
};

pub fn process_messenger(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    // load accounts
    let [
        sender,
        sender_fee_vault,
        global_config_account,
        _system_program,
        _others @ ..,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Audio is instruction data, validate checks
    let _voice_packet =
        VoicePacket::parse(data).map_err(|_| ProgramError::InvalidInstructionData)?;

    // signer checks
    if !sender.is_signer() {
        return Err(Nik8ProgramError::CreatorMustSign.into());
    }
    if !sender.is_writable() || !sender_fee_vault.is_writable() {
        return Err(ProgramError::Immutable);
    }
    if _system_program.address() != &pinocchio_system::ID {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    let fee_amount = validate_global_config(global_config_account)?;
    validate_sender_fee_vault(sender, sender_fee_vault)?;

    // The sender's PDA is a per-sender fee vault.
    Transfer {
        from: sender,
        lamports: fee_amount,
        to: sender_fee_vault,
    }
    .invoke()?;

    // do nothing with opus bytes, will be read offchain

    Ok(())
}

fn validate_global_config(global_config: &AccountView) -> Result<u64, ProgramError> {
    if !global_config.owned_by(&crate::ID) {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if global_config.data_len() != GlobalConfig::LEN {
        return Err(Nik8ProgramError::InvalidAccountLength.into());
    }

    // let (expected_config, expected_bump) =
    //     Address::derive_program_address(&[b"nik8_config"], &crate::ID)
    //         .ok_or(ProgramError::InvalidSeeds)?;
    // if global_config.address() != &expected_config {
    //     return Err(Nik8ProgramError::InvalidAccount.into());
    // }

    let config =
        unsafe { bytemuck::checked::from_bytes::<GlobalConfig>(global_config.borrow_unchecked()) };
    // if config.bump != expected_bump {
    //     return Err(Nik8ProgramError::InvalidAccount.into());
    // }

    Ok(u64::from_le_bytes(config.fee_amount))
}

fn validate_sender_fee_vault(sender: &AccountView, vault: &AccountView) -> ProgramResult {
    if !vault.owned_by(&crate::ID) {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if vault.data_len() != SenderAccount::LEN {
        return Err(Nik8ProgramError::InvalidAccountLength.into());
    }

    let vault_state =
        unsafe { bytemuck::checked::from_bytes::<SenderAccount>(vault.borrow_unchecked()) };
    if vault_state.owner_address != sender.address().to_bytes() {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    Ok(())
}
