use borsh::{BorshDeserialize, BorshSerialize};
use solana_pubkey::Pubkey;
use std::io;

use crate::oxedium::utils::ANCHOR_DISCRIMINATOR_LEN;

#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub struct PriceFeedMessage {
    pub feed_id: [u8; 32],
    pub price: i64,
    pub conf: u64,
    pub exponent: i32,
    pub prev_publish_time: i64,
    pub publish_time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum VerificationLevel {
    Partial { num_signatures: u8 },
    Full,
}

impl VerificationLevel {
    pub fn gte(&self, other: VerificationLevel) -> bool {
        match self {
            VerificationLevel::Full => true,
            VerificationLevel::Partial { num_signatures } => match other {
                VerificationLevel::Full => false,
                VerificationLevel::Partial {
                    num_signatures: other,
                } => *num_signatures >= other,
            },
        }
    }
}

#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub struct PriceUpdateV2 {
    pub write_authority: Pubkey,
    pub verification_level: VerificationLevel,
    pub price_message: PriceFeedMessage,
    pub posted_slot: u64,
}

impl PriceUpdateV2 {
    pub const LEN: usize = 32 + 1 + 32 + 8 + 8 + 4 + 8 + 8 + 8 + 8;

    pub fn try_from_account_data(data: &[u8]) -> Result<Self, io::Error> {
        let mut data = &data[ANCHOR_DISCRIMINATOR_LEN..];
        if data.len() < Self::LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Account data too short",
            ));
        }
        Self::deserialize(&mut data)
    }
}
