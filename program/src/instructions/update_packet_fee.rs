use pinocchio::{AccountView, ProgramResult, error::ProgramError};

use crate::{errors::Nik8ProgramError, state::GlobalConfig};

/// Updates the lamport fee charged for each valid `OnewayMessage` packet.
///
/// Accounts: configuration authority signer, writable global config.
/// Data: `new_fee_lamports: u64 LE`.
pub fn process_update_packet_fee(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [configuration_authority, global_config, _others @ ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let new_fee = parse_fee(data)?;

    if !configuration_authority.is_signer() {
        return Err(Nik8ProgramError::CreatorMustSign.into());
    }
    if !global_config.is_writable() {
        return Err(ProgramError::Immutable);
    }
    if !global_config.owned_by(&crate::ID) {
        return Err(Nik8ProgramError::InvalidAccount.into());
    }
    if global_config.data_len() != GlobalConfig::LEN {
        return Err(Nik8ProgramError::InvalidAccountLength.into());
    }

    let config =
        unsafe { bytemuck::checked::from_bytes::<GlobalConfig>(global_config.borrow_unchecked()) };
    if config.owner_address != configuration_authority.address().to_bytes() {
        return Err(Nik8ProgramError::UnauthorizedAuthority.into());
    }

    let config_data = unsafe { global_config.borrow_unchecked_mut() };
    config_data[1..9].copy_from_slice(&new_fee.to_le_bytes());

    Ok(())
}

fn parse_fee(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }

    Ok(u64::from_le_bytes(
        data.try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_fee;

    #[test]
    fn parses_an_eight_byte_fee() {
        assert_eq!(parse_fee(&123_u64.to_le_bytes()).unwrap(), 123);
    }

    #[test]
    fn rejects_an_incorrect_fee_length() {
        assert!(parse_fee(&[0; 7]).is_err());
        assert!(parse_fee(&[0; 9]).is_err());
    }
}
