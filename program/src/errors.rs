use pinocchio::error::ProgramError;

/// Program-specific errors. These values are returned to clients as
/// `ProgramError::Custom(code)`.
#[repr(u32)]
pub enum Nik8ProgramError {
    CreatorMustSign = 6_001,
    OracleMustBeSystemOwned = 6_002,
    AccountAlreadyInitialized = 6_003,
    InvalidAccountLength = 6_004,
    InvalidAuthority = 6_005,
    UnauthorizedAuthority = 6_006,
    InvalidAuthorityPda = 6_007,
    InvalidAccount = 6_008,
    RentCalculationFailure = 6_009,
    InstructionNotImplemented = 6_010,
}

impl From<Nik8ProgramError> for ProgramError {
    fn from(error: Nik8ProgramError) -> Self {
        ProgramError::Custom(error as u32)
    }
}
