#![allow(unexpected_cfgs)]
use pinocchio::{
    AccountView, Address, ProgramResult, address::declare_id, entrypoint, error::ProgramError,
};

use crate::instructions::{
    Nik8Ix, process_admin_withdraw_fee, process_create_account, process_initialize_program,
    process_messenger, process_update_packet_fee,
};

mod errors;
mod instructions;
mod state;
#[cfg(test)]
mod tests;

entrypoint!(process_instruction);

declare_id!("NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU");

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    if program_id != &crate::ID {
        return Err(ProgramError::IncorrectProgramId);
    }

    let (discriminator, data) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match Nik8Ix::try_from(discriminator)? {
        Nik8Ix::Initialize => process_initialize_program(accounts, data),
        Nik8Ix::OnewayMessage => process_messenger(accounts, data),
        Nik8Ix::Cre8Account => process_create_account(accounts, data),
        Nik8Ix::AdminWithdrawFee => process_admin_withdraw_fee(accounts, data),
        Nik8Ix::UpdatePacketFee => process_update_packet_fee(accounts, data),
    }
}
