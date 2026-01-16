use thiserror::Error;

#[derive(Debug, Error)]
pub enum OxediumVenueError {
    // ---------- Lifecycle ----------
    #[error("venue is not initialized")]
    NotInitialized,

    // ---------- State lookup ----------
    #[error("vault not found")]
    VaultNotFound,

    #[error("mint not found")]
    MintNotFound,

    #[error("oracle not found")]
    OracleNotFound,

    #[error("treasury not found")]
    TreasuryNotFound,

    #[error("account not found")]
    AccountNotFound,

    // ---------- Serialization ----------
    #[error("failed to deserialize account data")]
    DeserializationError,

    #[error("failed to serialize instruction data")]
    SerializationError,

    // ---------- Math / liquidity ----------
    #[error("not enough liquidity")]
    NotEnoughLiquidity,

    #[error("swap math error")]
    SwapMathError,

    // ---------- Generic ----------
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
}
