use pinocchio::{
    AccountView, Address, ProgramResult,
    error::ProgramError,
    sysvars::{Sysvar, rent::Rent},
};

use crate::{
    errors::Nik8ProgramError,
    state::{GlobalConfig, SenderAccount},
};

const WITHDRAW_DATA_LEN: usize = 8;

/// Sweeps accrued packet fees from one sender vault, preserving rent-exempt balance.
pub fn process_admin_withdraw_fee(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [
        fee_withdraw_authority,
        global_config,
        sender_fee_vault,
        destination,
        _others @ ..,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    let amount = parse_withdraw_amount(data)?;

    if !fee_withdraw_authority.is_signer() {
        return Err(Nik8ProgramError::UnauthorizedAuthority.into());
    }
    if !sender_fee_vault.is_writable() || !destination.is_writable() {
        return Err(ProgramError::Immutable);
    }
    if sender_fee_vault.address() == destination.address() {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    let withdraw_authority = load_withdraw_authority(global_config)?;
    if withdraw_authority != fee_withdraw_authority.address().to_bytes() {
        return Err(Nik8ProgramError::UnauthorizedAuthority.into());
    }

    validate_sender_fee_vault(sender_fee_vault)?;

    let rent_exempt_minimum = Rent::get()?
        .try_minimum_balance(SenderAccount::LEN)
        .map_err(|_| Nik8ProgramError::RentCalculationFailure)?;
    let remaining_lamports = sender_fee_vault
        .lamports()
        .checked_sub(amount)
        .ok_or(ProgramError::InsufficientFunds)?;
    if remaining_lamports < rent_exempt_minimum {
        return Err(ProgramError::InsufficientFunds);
    }
    let destination_lamports = destination
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    sender_fee_vault.set_lamports(remaining_lamports);
    destination.set_lamports(destination_lamports);

    Ok(())
}

fn parse_withdraw_amount(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() != WITHDRAW_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }

    let amount = u64::from_le_bytes(
        data.try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    if amount == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }

    Ok(amount)
}

fn load_withdraw_authority(global_config: &AccountView) -> Result<[u8; 32], ProgramError> {
    if !global_config.owned_by(&crate::ID) {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if global_config.data_len() != GlobalConfig::LEN {
        return Err(Nik8ProgramError::InvalidAccountLength.into());
    }

    let config =
        unsafe { bytemuck::checked::from_bytes::<GlobalConfig>(global_config.borrow_unchecked()) };

    Ok(config.fee_account_authority)
}

fn validate_sender_fee_vault(vault: &AccountView) -> ProgramResult {
    if !vault.owned_by(&crate::ID) {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if vault.data_len() != SenderAccount::LEN {
        return Err(Nik8ProgramError::InvalidAccountLength.into());
    }

    let vault_state =
        unsafe { bytemuck::checked::from_bytes::<SenderAccount>(vault.borrow_unchecked()) };
    let sender = Address::new_from_array(vault_state.owner_address);
    let seeds: [&[u8]; 2] = [b"nik8_user", sender.as_array()];
    let (expected_vault, expected_bump) =
        Address::derive_program_address(&seeds, &crate::ID).ok_or(ProgramError::InvalidSeeds)?;
    if vault.address() != &expected_vault || vault_state.bump != expected_bump {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_withdraw_amount;

    #[test]
    fn parses_a_nonzero_withdrawal_amount() {
        assert_eq!(parse_withdraw_amount(&42_u64.to_le_bytes()).unwrap(), 42);
    }

    #[test]
    fn rejects_zero_or_wrong_length_withdrawal_amount() {
        assert!(parse_withdraw_amount(&0_u64.to_le_bytes()).is_err());
        assert!(parse_withdraw_amount(&[0; 7]).is_err());
        assert!(parse_withdraw_amount(&[0; 9]).is_err());
    }
}
