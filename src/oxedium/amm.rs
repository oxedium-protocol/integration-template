use crate::{
    account_caching::AccountsCache,
    oxedium::{
        components::compute_swap_math,
        states::{PriceUpdateV2, SwapIxData, Treasury, Vault},
        utils::{ANCHOR_DISCRIMINATOR_LEN, OXEDIUM_SEED, TREASURY_SEED, VAULT_SEED},
    },
    trading_venue::{
        FromAccount, QuoteRequest, QuoteResult, TradingVenue,
        error::{ErrorInfo, TradingVenueError},
        protocol::PoolProtocol,
        token_info::TokenInfo,
    },
};
use ahash::{HashMap, HashMapExt};
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_sdk::system_program;
use spl_associated_token_account::get_associated_token_address;
use spl_token::state::Mint;

pub const OXEDIUM_AMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("oxe1SKL52HMLBDT2JQvdxscA1LbVc4EEwwSdNZcnDVH");

pub const MINT_ORACLES: &[(Pubkey, Pubkey)] = &[
    (
        Pubkey::from_str_const("So11111111111111111111111111111111111111112"),
        Pubkey::from_str_const("7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE"),
    ),
    (
        Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
        Pubkey::from_str_const("Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX"),
    ),
];

#[inline]
pub fn oracle_for_mint(mint: &Pubkey) -> Option<Pubkey> {
    MINT_ORACLES
        .iter()
        .find(|(m, _)| m == mint)
        .map(|(_, o)| *o)
}

pub struct OxediumAmmVenue {
    /// Titan lifecycle
    initialized: bool,

    pub vaults: HashMap<Pubkey, Vault>,
    pub mints: HashMap<Pubkey, Mint>,
    pub oracles: HashMap<Pubkey, PriceUpdateV2>,
    pub treasury: Treasury,

    pub token_infos: Vec<TokenInfo>,

    /// Market id (deterministic)
    pub market: Pubkey,
}

impl FromAccount for OxediumAmmVenue {
    fn from_account(pubkey: &Pubkey, _: &Account) -> Result<Self, TradingVenueError> {
        let initialized = false;
        let vaults = HashMap::new();
        let mints = HashMap::new();
        let oracles = HashMap::new();

        let treasury = Treasury {
            stoptap: false,
            admin: Pubkey::new_unique(),
            fee_bps: 0,
        };

        Ok(Self {
            initialized,
            vaults,
            mints,
            oracles,
            treasury,
            token_infos: vec![],
            market: *pubkey,
        })
    }
}

#[async_trait]
impl TradingVenue for OxediumAmmVenue {
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn program_id(&self) -> Pubkey {
        OXEDIUM_AMM_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![OXEDIUM_AMM_PROGRAM_ID]
    }

    fn market_id(&self) -> Pubkey {
        self.market
    }

    fn get_token_info(&self) -> &[TokenInfo] {
        &self.token_infos
    }

    fn protocol(&self) -> PoolProtocol {
        PoolProtocol::Oxedium
    }

    fn tradable_mints(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(MINT_ORACLES.iter().map(|(mint, _)| *mint).collect())
    }

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        let mut keys = Vec::new();

        for (mint, oracle) in MINT_ORACLES.iter() {
            let vault = Pubkey::find_program_address(
                &[VAULT_SEED.as_bytes(), mint.as_ref()],
                &self.program_id(),
            )
            .0;
            keys.push(vault);
            keys.push(*mint);
            keys.push(*oracle);
        }

        Ok(keys)
    }

    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<(), TradingVenueError> {
        let pubkeys = self.get_required_pubkeys_for_update()?;
        let accounts = cache.get_accounts(&pubkeys).await?;

        let account_map: HashMap<Pubkey, &Account> = pubkeys
            .iter()
            .zip(accounts.iter())
            .filter_map(|(pk, acc)| acc.as_ref().map(|a| (*pk, a)))
            .collect();

        for (mint, _) in MINT_ORACLES.iter() {
            let vault_pda = Pubkey::find_program_address(
                &[VAULT_SEED.as_bytes(), mint.as_ref()],
                &self.program_id(),
            )
            .0;

            if let Some(vault_account) = account_map.get(&vault_pda) {
                if vault_account.data.len() >= std::mem::size_of::<Vault>() {
                    if let Ok(vault) = Vault::deserialize(&mut &vault_account.data[ANCHOR_DISCRIMINATOR_LEN..]) {
                        self.vaults.insert(*mint, vault);
                    } else {
                        println!(">>> warning: failed to deserialize vault {:?}", vault_pda);
                    }
                } else {
                    println!(">>> warning: vault account data too small {:?}", vault_pda);
                }
            }

            if let Some(mint_account) = account_map.get(mint) {
                if mint_account.data.len() >= spl_token::state::Mint::LEN {
                    if let Ok(mint_data) = Mint::unpack(&mint_account.data) {
                        self.mints.insert(*mint, mint_data);
                    } else {
                        println!(">>> warning: failed to unpack mint {:?}", mint);
                    }
                } else {
                    println!(">>> warning: mint account data too small {:?}", mint);
                }
            }
        }

        for vault in self.vaults.values() {
            if let Some(oracle_account) = account_map.get(&vault.pyth_price_account) {
                if let Ok(price_data) = PriceUpdateV2::try_from_account_data(&oracle_account.data) {
                    self.oracles.insert(vault.pyth_price_account, price_data);
                } else {
                    println!(
                        ">>> warning: failed to deserialize oracle {:?}",
                        vault.pyth_price_account
                    );
                }
            }
        }

        self.token_infos = self
            .mints
            .iter()
            .map(|(mint_pubkey, mint_data)| TokenInfo {
                pubkey: *mint_pubkey,
                decimals: mint_data.decimals as i32,
                is_token_2022: false,
                transfer_fee: None,
                maximum_fee: None,
            })
            .collect();

        self.initialized = true;
        Ok(())
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        if !self.initialized {
            return Err(TradingVenueError::NotInitialized(ErrorInfo::StaticStr(
                "venue not initialized",
            )));
        }

        let vault_in = self.vaults.get(&request.input_mint).ok_or_else(|| {
            TradingVenueError::VaultNotFound(ErrorInfo::Pubkey(request.input_mint))
        })?;

        let vault_out = self.vaults.get(&request.output_mint).ok_or_else(|| {
            TradingVenueError::VaultNotFound(ErrorInfo::Pubkey(request.output_mint))
        })?;

        let in_mint = self
            .mints
            .get(&request.input_mint)
            .ok_or_else(|| TradingVenueError::InvalidMint(ErrorInfo::Pubkey(request.input_mint)))?;

        let out_mint = self.mints.get(&request.output_mint).ok_or_else(|| {
            TradingVenueError::InvalidMint(ErrorInfo::Pubkey(request.output_mint))
        })?;

        let price_in_data = self.oracles.get(&vault_in.pyth_price_account)
            .ok_or(TradingVenueError::OracleNotFound)?;
        print!("PRICE IN: {}\n", price_in_data.price_message.price);

        let price_out_data = self.oracles.get(&vault_out.pyth_price_account)
            .ok_or(TradingVenueError::OracleNotFound)?;
        print!("PRICE OUT: {}\n", price_out_data.price_message.price);

        let result = compute_swap_math(
            request.amount,
            price_in_data.price_message.price as u64,
            price_out_data.price_message.price as u64,
            in_mint.decimals,
            out_mint.decimals,
            vault_in,
            vault_out,
            &self.treasury,
        ).map_err(|e| TradingVenueError::MathError(ErrorInfo::String(format!("{e:?}"))))?;

        let mut not_enough_liquidity = false;
        if result.net_amount_out > vault_out.current_liquidity {
            not_enough_liquidity = true
        }

        Ok(QuoteResult {
            input_mint: request.input_mint,
            output_mint: request.output_mint,
            amount: request.amount,
            expected_output: result.net_amount_out,
            not_enough_liquidity: not_enough_liquidity,
        })
    }

    fn generate_swap_instruction(
        &self,
        request: QuoteRequest,
        user: Pubkey,
    ) -> Result<Instruction, TradingVenueError> {
        let user_in_ata = get_associated_token_address(&user, &request.input_mint);
        let user_out_ata = get_associated_token_address(&user, &request.output_mint);

        let treasury_pda = Pubkey::find_program_address(
            &[OXEDIUM_SEED.as_bytes(), TREASURY_SEED.as_bytes()],
            &OXEDIUM_AMM_PROGRAM_ID,
        ).0;

        let treasury_in_ata = get_associated_token_address(&treasury_pda, &request.input_mint);
        let treasury_out_ata = get_associated_token_address(&treasury_pda, &request.output_mint);

        let vault_in = Pubkey::find_program_address(
            &[VAULT_SEED.as_bytes(), request.input_mint.as_ref()],
            &OXEDIUM_AMM_PROGRAM_ID,
        ).0;

        let vault_out = Pubkey::find_program_address(
            &[VAULT_SEED.as_bytes(), request.output_mint.as_ref()],
            &OXEDIUM_AMM_PROGRAM_ID,
        ).0;

        let oracle_in = oracle_for_mint(&request.input_mint).ok_or(TradingVenueError::OracleNotFound)?;
        let oracle_out = oracle_for_mint(&request.output_mint).ok_or(TradingVenueError::OracleNotFound)?;

        let accounts = vec![
            AccountMeta::new(user, true),
            AccountMeta::new_readonly(request.input_mint, false),
            AccountMeta::new_readonly(request.output_mint, false),
            AccountMeta::new_readonly(oracle_in, false),
            AccountMeta::new_readonly(oracle_out, false),
            AccountMeta::new(user_in_ata, false),
            AccountMeta::new(user_out_ata, false),
            AccountMeta::new(vault_in, false),
            AccountMeta::new(vault_out, false),
            AccountMeta::new(treasury_pda, false),
            AccountMeta::new(treasury_in_ata, false),
            AccountMeta::new(treasury_out_ata, false),
            AccountMeta::new_readonly(spl_associated_token_account::ID, false),
            AccountMeta::new_readonly(spl_token::ID, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ];

        const DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
        let mut data = Vec::with_capacity(8 + 16);
        data.extend_from_slice(&DISCRIMINATOR);

        SwapIxData {
            amount_in: request.amount,
            min_amount_out: 1,
        }.serialize(&mut data)
            .map_err(|_| TradingVenueError::DeserializationError)?;

        Ok(Instruction {
            program_id: OXEDIUM_AMM_PROGRAM_ID,
            accounts,
            data,
        })
    }

    fn decimals(&self) -> Result<Vec<i32>, TradingVenueError> {
        Ok(self.get_token_info().iter().map(|x| x.decimals).collect())
    }

    fn get_token(&self, i: usize) -> Result<&TokenInfo, TradingVenueError> {
        self.get_token_info()
            .get(i)
            .ok_or(TradingVenueError::TokenInfoIndexError(i))
    }

    fn label(&self) -> String {
        self.protocol().into()
    }
}
