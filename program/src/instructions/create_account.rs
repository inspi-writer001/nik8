use pinocchio::{
    AccountView, Address, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, rent::Rent},
};
use pinocchio_system::instructions::CreateAccount;

use crate::{errors::Nik8ProgramError, state::SenderAccount};

pub fn process_create_account(accounts: &mut [AccountView], _data: &[u8]) -> ProgramResult {
    if !_data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    let [creator, account_to_create, system_program, _others @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if !creator.is_signer() {
        return Err(Nik8ProgramError::CreatorMustSign.into());
    }
    if !creator.is_writable() || !account_to_create.is_writable() {
        return Err(ProgramError::Immutable);
    }
    if system_program.address() != &pinocchio_system::ID {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    if !account_to_create.owned_by(&pinocchio_system::ID) {
        return Err(Nik8ProgramError::OracleMustBeSystemOwned.into());
    }

    if !account_to_create.is_data_empty() {
        return Err(Nik8ProgramError::AccountAlreadyInitialized.into());
    }

    let pda_seeds: [&[u8]; 2] = [b"nik8_user", creator.address().as_array()];
    let (expected_vault, bump) = Address::derive_program_address(&pda_seeds, &crate::ID)
        .ok_or(ProgramError::InvalidSeeds)?;

    if expected_vault != *account_to_create.address() {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }

    let bump_seed = [bump];
    let seeds = [
        Seed::from(b"nik8_user"),
        Seed::from(creator.address().as_array()),
        Seed::from(&bump_seed),
    ];

    let signer = Signer::from(&seeds);

    let rent_exempt_amount = Rent::get()?
        .try_minimum_balance(SenderAccount::LEN)
        .map_err(|_| Nik8ProgramError::RentCalculationFailure)?;

    CreateAccount {
        from: creator,
        lamports: rent_exempt_amount,
        owner: &crate::ID,
        space: SenderAccount::LEN as u64,
        to: account_to_create,
    }
    .invoke_signed(&[signer])?;

    let borrowed_data = unsafe { account_to_create.borrow_unchecked_mut() };
    borrowed_data[0] = bump;
    borrowed_data[1..SenderAccount::LEN].copy_from_slice(&creator.address().to_bytes());

    Ok(())
}
