use bytemuck::{CheckedBitPattern, NoUninit};

pub use nik8_shared::VoicePacket;

#[repr(C)]
#[derive(NoUninit, CheckedBitPattern, Clone, Copy)]
pub struct GlobalConfig {
    pub bump: u8,
    pub fee_amount: [u8; 8], // u64
    /// Public key bytes for the configuration authority.
    pub owner_address: [u8; 32],
    /// Public key bytes permitted to sweep sender fee vaults.
    pub fee_account_authority: [u8; 32],
}

/// SenderAccount doubles as the sender's SOL fee vault. Only the admin can
/// later sweep its accumulated fee lamports.
#[repr(C)]
#[derive(NoUninit, CheckedBitPattern, Clone, Copy)]
pub struct SenderAccount {
    pub bump: u8,
    /// Public key bytes of the sender whose PDA this is.
    pub owner_address: [u8; 32],
}

impl SenderAccount {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

impl GlobalConfig {
    pub const LEN: usize = core::mem::size_of::<Self>();
}
