use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SwapIxData {
    pub amount_in: u64,
    pub min_amount_out: u64,
}