#[cfg(test)]
mod simulations {
    use litesvm::LiteSVM;
    use rand::Rng;
    use rstest::rstest;

    use solana_account::Account;
    use solana_account::WritableAccount;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_compute_budget::compute_budget::ComputeBudget;
    use solana_program::native_token::LAMPORTS_PER_SOL;
    use solana_program_pack::Pack;
    use solana_pubkey::{Pubkey, pubkey};
    use solana_sdk::signature::Keypair;
    use solana_sdk::signer::Signer;
    use solana_sysvar::clock::{self, Clock};
    use solana_transaction::Transaction;
    use std::str::FromStr;
    use std::time::Instant;

    use spl_associated_token_account::get_associated_token_address_with_program_id;
    use spl_token::state::{Account as TokenAccount, AccountState};

    use titan_integration_template::oxedium::amm::OXEDIUM_AMM_PROGRAM_ID;
    use titan_integration_template::trading_venue::SwapType;

    use titan_integration_template::{
        account_caching::AccountsCache, oxedium::amm::OxediumAmmVenue, trading_venue::QuoteRequest,
    };
    use titan_integration_template::{
        account_caching::rpc_cache::RpcClientCache,
        trading_venue::{FromAccount, TradingVenue, error::TradingVenueError},
    };

    const RPC_URL: &str = "https://api.mainnet-beta.solana.com";

    fn init_test_logger() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    pub fn setup_litesvm() -> (LiteSVM, Keypair) {
        let mut litesvm = LiteSVM::new().with_compute_budget(ComputeBudget {
            compute_unit_limit: 1_400_000,
            ..Default::default()
        });

        let spl_calc_program = pubkey!("oxe1SKL52HMLBDT2JQvdxscA1LbVc4EEwwSdNZcnDVH");
        let spl_calc_path = format!("programs/{}.so", spl_calc_program);
        litesvm
            .add_program_from_file(spl_calc_program, spl_calc_path)
            .unwrap();

        let spl_calc_program_2 = pubkey!("oxe1SKL52HMLBDT2JQvdxscA1LbVc4EEwwSdNZcnDVH");
        let spl_calc_path_2 = format!("programs/{}.so", spl_calc_program_2);
        litesvm
            .add_program_from_file(spl_calc_program_2, spl_calc_path_2)
            .unwrap();

        let keypair = Keypair::new();
        let account = Account {
            lamports: 10_000 * LAMPORTS_PER_SOL,
            data: vec![],
            owner: solana_sdk::system_program::id(),
            executable: false,
            rent_epoch: 0,
        };
        litesvm
            .set_account(keypair.pubkey(), account.into())
            .unwrap();

        (litesvm, keypair)
    }

    async fn sim_quote_request(
        venue: &dyn TradingVenue,
        cache: &dyn AccountsCache,
        request: QuoteRequest,
        litesvm: &mut LiteSVM,
        keypair: &Keypair,
    ) -> u64 {
        let tradable_mints = venue.get_token_info();

        let idx_0 = tradable_mints
            .iter()
            .position(|x| x.pubkey == request.input_mint)
            .unwrap();
        let idx_1 = (idx_0 + 1) % 2;

        let (token_a, token_a_program) = (
            tradable_mints[idx_0].pubkey,
            tradable_mints[idx_0].get_token_program(),
        );
        let (token_b, token_b_program) = (
            tradable_mints[idx_1].pubkey,
            tradable_mints[idx_1].get_token_program(),
        );

        let token_account_a = get_associated_token_address_with_program_id(
            &keypair.pubkey(),
            &token_a,
            &token_a_program,
        );
        let token_account_b = get_associated_token_address_with_program_id(
            &keypair.pubkey(),
            &token_b,
            &token_b_program,
        );

        let mut account_a = Account::new(LAMPORTS_PER_SOL, TokenAccount::LEN, &spl_token::ID);
        let mut account_a_data = TokenAccount::default();
        account_a_data.mint = token_a;
        account_a_data.owner = keypair.pubkey();
        account_a_data.state = AccountState::Initialized;
        account_a_data.amount = u64::MAX;
        account_a_data.pack_into_slice(account_a.data_as_mut_slice());

        let mut account_b = Account::new(LAMPORTS_PER_SOL, TokenAccount::LEN, &spl_token::ID);
        let mut account_b_data = TokenAccount::default();
        account_b_data.mint = token_b;
        account_b_data.owner = keypair.pubkey();
        account_b_data.state = AccountState::Initialized;
        account_b_data.amount = 0;
        account_b_data.pack_into_slice(account_b.data_as_mut_slice());

        litesvm.set_account(token_account_a, account_a).unwrap();
        litesvm.set_account(token_account_b, account_b).unwrap();

        let ix = venue
            .generate_swap_instruction(request, keypair.pubkey())
            .unwrap();

        let pks: Vec<Pubkey> = ix.accounts.iter().map(|acc| acc.pubkey).collect();
        let accounts_to_load = cache.get_accounts(&pks).await.unwrap();
        for (account, key) in accounts_to_load.iter().zip(pks) {
            if let Some(acc) = account {
                if acc.executable {
                    continue;
                }
                litesvm.set_account(key, acc.clone()).unwrap();
            }
        }

        let blockhash = litesvm.latest_blockhash();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&keypair.pubkey()),
            &[keypair],
            blockhash,
        );

        litesvm.send_transaction(tx).unwrap();

        let account_b = litesvm.get_account(&token_account_b).unwrap();
        let post_b = TokenAccount::unpack_from_slice(&account_b.data).unwrap();
        post_b.amount
    }

    fn sample_log_uniform_u64(lo: u64, hi: u64) -> u64 {
        let lo_f = lo as f64;
        let hi_f = hi as f64;
        let r: f64 = rand::rng().random();
        ((lo_f.ln() + r * (hi_f.ln() - lo_f.ln())).exp() as u64).clamp(lo, hi)
    }

    #[rstest]
    #[tokio::test]
    #[case("5hWhYNZ8HNJbzFAwMBso5ERBFrWZ7QnrEk7aQVhHDNv4")]
    async fn test_bound_simulation(#[case] amm_key: Pubkey) {
        init_test_logger();

        let rpc = RpcClient::new(RPC_URL.to_string());
        let venue_account = rpc.get_account(&amm_key).await.unwrap();

        let cache = RpcClientCache::new(rpc);
        let mut venue = OxediumAmmVenue::from_account(&amm_key, &venue_account).unwrap();
        venue.update_state(&cache).await.unwrap();

        let (mut litesvm, keypair) = setup_litesvm();

        litesvm
            .add_program_from_file(
                OXEDIUM_AMM_PROGRAM_ID,
                "programs/oxe1SKL52HMLBDT2JQvdxscA1LbVc4EEwwSdNZcnDVH.so",
            )
            .unwrap();

        let latest_clock = cache.get_account(&clock::ID).await.unwrap();
        let latest_clock: Clock = latest_clock
            .as_ref()
            .ok_or(TradingVenueError::NoAccountFound(clock::ID.into()))
            .unwrap()
            .deserialize_data()
            .unwrap();
        litesvm.set_sysvar::<Clock>(&latest_clock);

        for (in_idx, out_idx) in [(0, 1), (1, 0)] {
            let (lower, upper) = venue.bounds(in_idx as u8, out_idx as u8).unwrap();

            for bound in [lower, upper] {
                let request = QuoteRequest {
                    input_mint: venue.get_token(in_idx).unwrap().pubkey,
                    output_mint: venue.get_token(out_idx).unwrap().pubkey,
                    amount: bound,
                    swap_type: SwapType::ExactIn,
                };

                let sim =
                    sim_quote_request(&venue, &cache, request.clone(), &mut litesvm, &keypair)
                        .await;
                let quote = venue.quote(request).unwrap();

                assert_eq!(quote.expected_output.abs_diff(sim), 0)
            }
        }
    }

    #[rstest]
    #[tokio::test]
    #[case("5hWhYNZ8HNJbzFAwMBso5ERBFrWZ7QnrEk7aQVhHDNv4")]
    async fn test_random_samples(#[case] amm_key: Pubkey) {
        init_test_logger();

        let rpc = RpcClient::new(RPC_URL.to_string());
        let venue_account = rpc.get_account(&amm_key).await.unwrap();

        let cache = RpcClientCache::new(rpc);
        let mut venue = OxediumAmmVenue::from_account(&amm_key, &venue_account).unwrap();
        venue.update_state(&cache).await.unwrap();

        let (mut litesvm, keypair) = setup_litesvm();
        litesvm
            .add_program_from_file(
                OXEDIUM_AMM_PROGRAM_ID,
                "programs/oxe1SKL52HMLBDT2JQvdxscA1LbVc4EEwwSdNZcnDVH.so",
            )
            .unwrap();

        let latest_clock = cache.get_account(&clock::ID).await.unwrap();
        let latest_clock: Clock = latest_clock
            .as_ref()
            .ok_or(TradingVenueError::NoAccountFound(clock::ID.into()))
            .unwrap()
            .deserialize_data()
            .unwrap();
        litesvm.set_sysvar::<Clock>(&latest_clock);

        for (in_idx, out_idx) in [(0, 1), (1, 0)] {
            let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();

            for _ in 0..1 {
                let amount = sample_log_uniform_u64(lb, ub);

                let request = QuoteRequest {
                    input_mint: venue.get_token(in_idx as usize).unwrap().pubkey,
                    output_mint: venue.get_token(out_idx as usize).unwrap().pubkey,
                    amount,
                    swap_type: SwapType::ExactIn,
                };

                let sim =
                    sim_quote_request(&venue, &cache, request.clone(), &mut litesvm, &keypair)
                        .await;
                let quote = venue.quote(request).unwrap();

                assert_eq!(quote.expected_output.abs_diff(sim), 0)
            }
        }
    }

    #[rstest]
    #[tokio::test]
    #[case("5hWhYNZ8HNJbzFAwMBso5ERBFrWZ7QnrEk7aQVhHDNv4")]
    async fn test_monotone(#[case] amm_key: String) {
        init_test_logger();

        let amm_key = Pubkey::from_str(&amm_key).unwrap();
        let rpc = RpcClient::new(RPC_URL.to_string());

        let venue_account = rpc.get_account(&amm_key).await.unwrap();
        let mut venue = OxediumAmmVenue::from_account(&amm_key, &venue_account).unwrap();

        let cache = RpcClientCache::new(rpc);
        venue.update_state(&cache).await.unwrap();

        let token_info = venue.get_token_info();

        for (in_idx, out_idx) in [(0, 1), (1, 0)] {
            let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
            let mut amounts = (0..50)
                .map(|_| sample_log_uniform_u64(lb, ub))
                .collect::<Vec<_>>();
            amounts.sort();

            let mut prev = 0;
            for amount in amounts {
                let result = venue
                    .quote(QuoteRequest {
                        input_mint: token_info[in_idx as usize].pubkey,
                        output_mint: token_info[out_idx as usize].pubkey,
                        amount,
                        swap_type: SwapType::ExactIn,
                    })
                    .unwrap();

                assert!(prev <= result.expected_output);
                prev = result.expected_output;
            }
        }
    }

    #[rstest]
    #[tokio::test]
    #[case("5hWhYNZ8HNJbzFAwMBso5ERBFrWZ7QnrEk7aQVhHDNv4", 10_000)]
    async fn test_quoting_speed(#[case] amm_key: String, #[case] iterations: usize) {
        init_test_logger();

        let amm_key = Pubkey::from_str(&amm_key).unwrap();
        let rpc = RpcClient::new(RPC_URL.to_string());

        let venue_account = rpc.get_account(&amm_key).await.unwrap();
        let mut venue = OxediumAmmVenue::from_account(&amm_key, &venue_account).unwrap();

        let cache = RpcClientCache::new(rpc);
        venue.update_state(&cache).await.unwrap();

        let token_info = venue.get_token_info();

        for (in_idx, out_idx) in [(0, 1), (1, 0)] {
            let (lb, ub) = venue.bounds(in_idx, out_idx).unwrap();
            let amounts = (0..iterations)
                .map(|_| sample_log_uniform_u64(lb, ub))
                .collect::<Vec<_>>();

            let start = Instant::now();
            for amount in amounts {
                venue
                    .quote(QuoteRequest {
                        input_mint: token_info[in_idx as usize].pubkey,
                        output_mint: token_info[out_idx as usize].pubkey,
                        amount,
                        swap_type: SwapType::ExactIn,
                    })
                    .unwrap();
            }

            let avg = start.elapsed().as_secs_f64() / iterations as f64;
            assert!(avg < 0.0001);
        }
    }
}
