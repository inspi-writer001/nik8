use pinocchio::{
    AccountView, Address, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, rent::Rent},
};
use pinocchio_system::instructions::CreateAccount;

use crate::{errors::Nik8ProgramError, state::GlobalConfig};

const INITIALIZE_DATA_LEN: usize = 40;

/// Creates the singleton global configuration account.
///
/// Instruction data: `[fee_lamports: u64 LE | fee_withdraw_authority: [u8; 32]]`.
pub fn process_initialize_program(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [
        configuration_authority,
        global_config,
        system_program,
        _others @ ..,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    let (fee_amount, fee_withdraw_authority) = parse_initialize_data(data)?;

    if !configuration_authority.is_signer() {
        return Err(Nik8ProgramError::CreatorMustSign.into());
    }
    if !configuration_authority.is_writable() || !global_config.is_writable() {
        return Err(ProgramError::Immutable);
    }
    if system_program.address() != &pinocchio_system::ID {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if !global_config.owned_by(&pinocchio_system::ID) {
        return Err(Nik8ProgramError::OracleMustBeSystemOwned.into());
    }
    if !global_config.is_data_empty() {
        return Err(Nik8ProgramError::AccountAlreadyInitialized.into());
    }

    let (expected_config, bump) = Address::derive_program_address(&[b"nik8_config"], &crate::ID)
        .ok_or(ProgramError::InvalidSeeds)?;
    if global_config.address() != &expected_config {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    let bump_seed = [bump];
    let seeds = [Seed::from(b"nik8_config"), Seed::from(&bump_seed)];
    let signer = Signer::from(&seeds);

    let rent_exempt_amount = Rent::get()?
        .try_minimum_balance(GlobalConfig::LEN)
        .map_err(|_| Nik8ProgramError::RentCalculationFailure)?;

    CreateAccount {
        from: configuration_authority,
        lamports: rent_exempt_amount,
        owner: &crate::ID,
        space: GlobalConfig::LEN as u64,
        to: global_config,
    }
    .invoke_signed(&[signer])?;

    let config_data = unsafe { global_config.borrow_unchecked_mut() };
    config_data[0] = bump;
    config_data[1..9].copy_from_slice(&fee_amount.to_le_bytes());
    config_data[9..41].copy_from_slice(&configuration_authority.address().to_bytes());
    config_data[41..GlobalConfig::LEN].copy_from_slice(&fee_withdraw_authority);

    Ok(())
}

fn parse_initialize_data(data: &[u8]) -> Result<(u64, [u8; 32]), ProgramError> {
    if data.len() != INITIALIZE_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }

    let fee_amount = u64::from_le_bytes(
        data[..8]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    let fee_withdraw_authority = data[8..]
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    Ok((fee_amount, fee_withdraw_authority))
}

#[cfg(test)]
mod tests {
    use super::parse_initialize_data;

    #[test]
    fn parses_initialize_data() {
        let mut data = 42_u64.to_le_bytes().to_vec();
        data.extend_from_slice(&[9; 32]);

        assert_eq!(parse_initialize_data(&data).unwrap(), (42, [9; 32]));
    }

    #[test]
    fn rejects_wrong_length_initialize_data() {
        assert!(parse_initialize_data(&[0; 39]).is_err());
        assert!(parse_initialize_data(&[0; 41]).is_err());
    }
}
