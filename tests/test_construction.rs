#[cfg(test)]
mod test_construction {
    use std::str::FromStr;

    use rstest::rstest;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_pubkey::Pubkey;

    use titan_integration_template::account_caching::rpc_cache::RpcClientCache;
    use titan_integration_template::trading_venue::{QuoteRequest, SwapType};
    use titan_integration_template::{
        oxedium::amm::OxediumAmmVenue,
        trading_venue::{FromAccount, TradingVenue},
    };

    use assert_no_alloc::*;

    const RPC_URL: &str = "https://api.mainnet-beta.solana.com";

    fn init_test_logger() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[rstest]
    #[tokio::test]
    #[case("5hWhYNZ8HNJbzFAwMBso5ERBFrWZ7QnrEk7aQVhHDNv4")]
    async fn test_construction(#[case] amm_key: String) {
        init_test_logger();

        let amm_key = Pubkey::from_str(&amm_key).expect("Invalid test pubkey");

        let rpc = RpcClient::new(RPC_URL.to_string());

        let venue_account = rpc
            .get_account(&amm_key)
            .await
            .expect("Failed to fetch AMM account");

        let mut venue: OxediumAmmVenue =
            OxediumAmmVenue::from_account(&amm_key, &venue_account)
                .expect("Failed to construct venue from account");

        let cache = RpcClientCache::new(rpc);
        venue
            .update_state(&cache)
            .await
            .expect("Venue state update failed");

        let token_info = venue.get_token_info();
        assert!(token_info.len() > 0);
        assert_eq!(token_info.len(), 2);

        for (input_idx, output_idx) in [(0, 1), (1, 0)] {
            let (lower_bound, upper_bound) =
                assert_no_alloc(|| venue.bounds(input_idx, output_idx))
                    .expect("Boundary search failed");

            assert!(lower_bound < upper_bound);

            let input_mint = token_info[input_idx as usize].pubkey;
            let output_mint = token_info[output_idx as usize].pubkey;

            let lb_result = assert_no_alloc(|| {
                venue.quote(QuoteRequest {
                    input_mint,
                    output_mint,
                    amount: lower_bound,
                    swap_type: SwapType::ExactIn,
                })
            })
            .expect("Lower-bound quote failed");

            assert!(!lb_result.not_enough_liquidity);
            assert!(lb_result.expected_output > 0);

            let ub_result = assert_no_alloc(|| {
                venue.quote(QuoteRequest {
                    input_mint,
                    output_mint,
                    amount: upper_bound,
                    swap_type: SwapType::ExactIn,
                })
            })
            .expect("Upper-bound quote failed");

            assert!(!ub_result.not_enough_liquidity);
            assert!(ub_result.expected_output > 0);
        }
    }
}
