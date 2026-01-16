use solana_pubkey::Pubkey;
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Clone, Copy, Debug, Default, BorshDeserialize, BorshSerialize)]
pub struct Treasury {
    pub stoptap: bool,
    pub admin: Pubkey,
    pub fee_bps: u64
}