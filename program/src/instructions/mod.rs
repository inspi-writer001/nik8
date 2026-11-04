use pinocchio::error::ProgramError;
pub mod admin_withdraw_fee;
pub mod create_account;
pub mod initialize_program;
pub mod messenger;
pub mod update_packet_fee;

pub use admin_withdraw_fee::*;
pub use create_account::*;
pub use initialize_program::*;
pub use messenger::*;
pub use update_packet_fee::*;

#[non_exhaustive]
pub enum Nik8Ix {
    Initialize = 0,
    OnewayMessage = 1,
    Cre8Account = 2,
    AdminWithdrawFee = 3,
    UpdatePacketFee = 4,
}

impl TryFrom<&u8> for Nik8Ix {
    type Error = ProgramError;

    fn try_from(value: &u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Nik8Ix::Initialize),
            1 => Ok(Nik8Ix::OnewayMessage),
            2 => Ok(Nik8Ix::Cre8Account),
            3 => Ok(Nik8Ix::AdminWithdrawFee),
            4 => Ok(Nik8Ix::UpdatePacketFee),

            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}
