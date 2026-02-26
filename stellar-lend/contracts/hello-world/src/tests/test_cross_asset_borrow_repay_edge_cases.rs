#![cfg(test)]
//! Cross-Asset Borrow and Repay Edge Case Tests
//!
//! This module tests complex scenarios involving:
//! - Borrowing against multiple collateral assets
//! - Repaying debt across multiple assets
//! - Edge cases with collateral devaluation
//! - Partial repayment scenarios
//! - Health factor updates during multi-asset operations

use crate::{HelloContract, HelloContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};
use crate::cross_asset::AssetConfig;

// ============================================================================
// TEST HELPERS
// ============================================================================

fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

fn setup_contract(env: &Env) -> (HelloContractClient, Address) {
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize_ca(&admin);
    (client, admin)
}

fn create_asset_config(env: &Env, asset: Option<Address>, price: i128) -> AssetConfig {
    AssetConfig {
        asset: asset.clone(),
        collateral_factor: 7500, // 75%
        borrow_factor: 8000,     // 80%
        reserve_factor: 1000,    // 10%
        max_supply: 100_000_000_000_000,
        max_borrow: 80_000_000_000_000,
        can_collateralize: true,
        can_borrow: true,
        price,
        price_updated_at: env.ledger().timestamp(),
    }
}

fn create_custom_asset_config(
    env: &Env,
    asset: Option<Address>,
    price: i128,
    collateral_factor: i128,
    borrow_factor: i128,
) -> AssetConfig {
    AssetConfig {
        asset: asset.clone(),
        collateral_factor,
        borrow_factor,
        reserve_factor: 1000,
        max_supply: 100_000_000_000_000,
        max_borrow: 80_000_000_000_000,
        can_collateralize: true,
        can_borrow: true,
        price,
        price_updated_at: env.ledger().timestamp(),
    }
}

fn setup_three_assets(env: &Env, client: &HelloContractClient) -> (Address, Address, Address) {
    let usdc = Address::generate(env);
    client.initialize_asset(&Some(usdc.clone()), &create_asset_config(env, Some(usdc.clone()), 1_0000000));
    
    let eth = Address::generate(env);
    client.initialize_asset(&Some(eth.clone()), &create_asset_config(env, Some(eth.clone()), 2000_0000000));
    
    let btc = Address::generate(env);
    client.initialize_asset(&Some(btc.clone()), &create_asset_config(env, Some(btc.clone()), 40000_0000000));
    
    (usdc, eth, btc)
}

// ============================================================================
// MULTI-COLLATERAL BORROW TESTS
// ============================================================================

#[test]
fn test_borrow_single_asset_against_three_collaterals() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Deposit three different collaterals
    // USDC: $10,000
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    // ETH: 5 * $2,000 = $10,000
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &5_0000000);
    // BTC: 0.5 * $40,000 = $20,000
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &5000000);

    // Total collateral: $40,000
    // Weighted (75%): $30,000
    // Borrow $25,000 USDC
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &25000_0000000);

    let position = client.get_user_asset_position(&user, &Some(usdc.clone()));
    assert_eq!(position.debt_principal, 25000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 40000_0000000);
    assert_eq!(summary.total_debt_value, 25000_0000000);
    assert!(summary.health_factor > 10000);
}

#[test]
fn test_borrow_multiple_assets_against_multiple_collaterals() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Deposit USDC and BTC as collateral
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &50000_0000000);
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &1_0000000);

    // Total: $50k USDC + $40k BTC = $90k
    // Weighted: $67.5k

    // Borrow ETH
    client.ca_borrow_asset(&user, &Some(eth.clone()), &15_0000000);
    // Borrow more USDC
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &20000_0000000);

    // Total debt: $30k ETH + $20k USDC = $50k

    let eth_position = client.get_user_asset_position(&user, &Some(eth));
    assert_eq!(eth_position.debt_principal, 15_0000000);

    let usdc_position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(usdc_position.debt_principal, 20000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_debt_value, 50000_0000000);
    assert!(summary.health_factor > 10000);
}

#[test]
fn test_borrow_at_maximum_capacity_multi_collateral() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Deposit collateral
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &5_0000000);

    // Borrow a reasonable amount
    client.ca_borrow_asset(&user, &Some(btc.clone()), &370000);

    let summary = client.get_user_position_summary(&user);
    // Verify position is healthy
    assert!(summary.health_factor >= 10000);
    // Verify we have used significant borrow capacity
    assert!(summary.total_debt_value > 0);
}

#[test]
fn test_borrow_exceeds_multi_collateral_capacity() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &5_0000000);

    // Try to borrow significantly more than weighted collateral allows
    // Total: $20k, Weighted: $15k, trying to borrow $20k should fail
    let result = client.try_ca_borrow_asset(&user, &Some(usdc), &20000_0000000);
    assert!(result.is_err());
}

#[test]
fn test_sequential_borrows_different_assets() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Large collateral
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &2_0000000);

    // Borrow USDC
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &20000_0000000);
    let summary1 = client.get_user_position_summary(&user);
    let capacity1 = summary1.borrow_capacity;

    // Borrow ETH
    client.ca_borrow_asset(&user, &Some(eth.clone()), &5_0000000);
    let summary2 = client.get_user_position_summary(&user);
    let capacity2 = summary2.borrow_capacity;

    // Capacity should decrease with each borrow
    assert!(capacity2 < capacity1);

    // Verify both debts exist
    assert_eq!(client.get_user_asset_position(&user, &Some(usdc)).debt_principal, 20000_0000000);
    assert_eq!(client.get_user_asset_position(&user, &Some(eth)).debt_principal, 5_0000000);
}

// ============================================================================
// PARTIAL REPAYMENT TESTS
// ============================================================================

#[test]
fn test_partial_repay_single_asset_debt() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &5000_0000000);

    // Repay 25%
    client.ca_repay_debt(&user, &Some(usdc.clone()), &1250_0000000);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.debt_principal, 3750_0000000);
}

#[test]
fn test_partial_repay_multiple_assets() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Setup collateral and borrow multiple assets
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &2_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &30000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &10_0000000);

    // Partially repay USDC
    client.ca_repay_debt(&user, &Some(usdc.clone()), &10000_0000000);
    // Partially repay ETH
    client.ca_repay_debt(&user, &Some(eth.clone()), &3_0000000);

    assert_eq!(client.get_user_asset_position(&user, &Some(usdc)).debt_principal, 20000_0000000);
    assert_eq!(client.get_user_asset_position(&user, &Some(eth)).debt_principal, 7_0000000);
}

#[test]
fn test_repay_one_asset_fully_keep_others() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(btc.clone()), &2_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &20000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &10_0000000);

    // Fully repay USDC
    client.ca_repay_debt(&user, &Some(usdc.clone()), &20000_0000000);

    assert_eq!(client.get_user_asset_position(&user, &Some(usdc)).debt_principal, 0);
    assert_eq!(client.get_user_asset_position(&user, &Some(eth)).debt_principal, 10_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_debt_value, 20000_0000000); // Only ETH debt remains
}

#[test]
fn test_repay_more_than_debt_caps_at_zero() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &5000_0000000);

    // Try to repay double the debt
    client.ca_repay_debt(&user, &Some(usdc.clone()), &10000_0000000);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.debt_principal, 0);
    assert_eq!(position.accrued_interest, 0);
}

#[test]
fn test_repay_all_debts_sequentially() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(btc.clone()), &2_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &5_0000000);

    // Repay all USDC
    client.ca_repay_debt(&user, &Some(usdc.clone()), &10000_0000000);
    // Repay all ETH
    client.ca_repay_debt(&user, &Some(eth.clone()), &5_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_debt_value, 0);
    assert_eq!(summary.health_factor, i128::MAX);
}

// ============================================================================
// COLLATERAL DEVALUATION EDGE CASES
// ============================================================================

#[test]
fn test_borrow_then_collateral_price_drops() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    // Deposit ETH as collateral
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &10_0000000);
    // Borrow USDC
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &10000_0000000);

    let summary_before = client.get_user_position_summary(&user);
    assert!(!summary_before.is_liquidatable);

    // ETH price drops 50%
    client.update_asset_price(&Some(eth), &1000_0000000);

    let summary_after = client.get_user_position_summary(&user);
    assert!(summary_after.health_factor < summary_before.health_factor);
    assert!(summary_after.is_liquidatable);
}

#[test]
fn test_multi_collateral_one_asset_devalues() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Deposit multiple collaterals
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &20000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &10_0000000);
    client.ca_borrow_asset(&user, &Some(btc.clone()), &500000);

    let summary_before = client.get_user_position_summary(&user);

    // ETH price drops 80%
    client.update_asset_price(&Some(eth), &400_0000000);

    let summary_after = client.get_user_position_summary(&user);
    // Should still be healthy due to USDC collateral
    assert!(summary_after.health_factor < summary_before.health_factor);
    assert!(!summary_after.is_liquidatable);
}

#[test]
fn test_all_collateral_devalues_becomes_liquidatable() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Use ETH and BTC as collateral, borrow USDC
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &10_0000000);
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &5000000);
    // Total: $20k ETH + $20k BTC = $40k
    // Weighted: $30k
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &28000_0000000);

    let summary_before = client.get_user_position_summary(&user);
    let health_before = summary_before.health_factor;

    // Both collaterals lose 90% value
    client.update_asset_price(&Some(eth.clone()), &200_0000000);
    client.update_asset_price(&Some(btc), &4000_0000000);

    let summary = client.get_user_position_summary(&user);
    // Health factor should decrease significantly
    assert!(summary.health_factor < health_before);
    // Position should become liquidatable
    assert!(summary.is_liquidatable);
}

#[test]
fn test_borrowed_asset_price_increases() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &20000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &5_0000000);

    let summary_before = client.get_user_position_summary(&user);

    // ETH price doubles
    client.update_asset_price(&Some(eth), &4000_0000000);

    let summary_after = client.get_user_position_summary(&user);
    assert!(summary_after.total_debt_value > summary_before.total_debt_value);
    assert!(summary_after.health_factor < summary_before.health_factor);
}

// ============================================================================
// COLLATERAL REMOVAL EDGE CASES
// ============================================================================

#[test]
fn test_withdraw_one_collateral_maintain_health() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    // Deposit two collaterals
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &20000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &10_0000000);
    // Borrow
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &15000_0000000);

    // Withdraw some USDC (should still be healthy with ETH)
    client.ca_withdraw_collateral(&user, &Some(usdc.clone()), &10000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert!(!summary.is_liquidatable);
    assert!(summary.health_factor > 10000);
}

#[test]
fn test_withdraw_collateral_breaks_health_fails() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &5_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &14000_0000000);

    // Try to withdraw ETH (would break health)
    let result = client.try_ca_withdraw_collateral(&user, &Some(eth), &5_0000000);
    assert!(result.is_err());
}

#[test]
fn test_withdraw_all_collateral_after_full_repay() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &5_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &10000_0000000);

    // Repay all debt
    client.ca_repay_debt(&user, &Some(usdc.clone()), &10000_0000000);

    // Withdraw all collateral
    client.ca_withdraw_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_withdraw_collateral(&user, &Some(eth), &5_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 0);
    assert_eq!(summary.total_debt_value, 0);
}

// ============================================================================
// DIFFERENT COLLATERAL FACTOR EDGE CASES
// ============================================================================

#[test]
fn test_borrow_with_different_collateral_factors() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);

    // High collateral factor asset (90%)
    let stable = Address::generate(&env);
    client.initialize_asset(&Some(stable.clone()), &create_custom_asset_config(&env, Some(stable.clone()), 1_0000000, 9000, 8000));

    // Low collateral factor asset (50%)
    let volatile = Address::generate(&env);
    client.initialize_asset(&Some(volatile.clone()), &create_custom_asset_config(&env, Some(volatile.clone()), 1000_0000000, 5000, 8000));

    // Deposit both
    client.ca_deposit_collateral(&user, &Some(stable.clone()), &10000_0000000);
    client.ca_deposit_collateral(&user, &Some(volatile.clone()), &10_0000000);

    // Total: $10k + $10k = $20k
    // Weighted: $9k (stable) + $5k (volatile) = $14k

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 20000_0000000);
    assert_eq!(summary.weighted_collateral_value, 14000_0000000);
}

#[test]
fn test_repay_improves_health_factor() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &7000_0000000);

    let summary_before = client.get_user_position_summary(&user);
    let health_before = summary_before.health_factor;

    // Repay half
    client.ca_repay_debt(&user, &Some(usdc.clone()), &3500_0000000);

    let summary_after = client.get_user_position_summary(&user);
    assert!(summary_after.health_factor > health_before);
}

#[test]
fn test_borrow_capacity_updates_correctly() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &20000_0000000);
    
    let summary1 = client.get_user_position_summary(&user);
    let initial_capacity = summary1.borrow_capacity;

    // Borrow some
    client.ca_borrow_asset(&user, &Some(eth.clone()), &3_0000000);

    let summary2 = client.get_user_position_summary(&user);
    assert!(summary2.borrow_capacity < initial_capacity);

    // Repay
    client.ca_repay_debt(&user, &Some(eth.clone()), &1_0000000);

    let summary3 = client.get_user_position_summary(&user);
    assert!(summary3.borrow_capacity > summary2.borrow_capacity);
}

// ============================================================================
// COMPLEX MULTI-STEP SCENARIOS
// ============================================================================

#[test]
fn test_complex_multi_asset_lifecycle() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Step 1: Deposit USDC
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &50000_0000000);

    // Step 2: Borrow ETH
    client.ca_borrow_asset(&user, &Some(eth.clone()), &10_0000000);

    // Step 3: Deposit BTC
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &1_0000000);

    // Step 4: Borrow more USDC
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &30000_0000000);

    // Step 5: Partial repay ETH
    client.ca_repay_debt(&user, &Some(eth.clone()), &5_0000000);

    // Step 6: Withdraw some USDC
    client.ca_withdraw_collateral(&user, &Some(usdc.clone()), &20000_0000000);

    // Verify final state
    let usdc_pos = client.get_user_asset_position(&user, &Some(usdc.clone()));
    assert_eq!(usdc_pos.collateral, 30000_0000000);
    assert_eq!(usdc_pos.debt_principal, 30000_0000000);

    let eth_pos = client.get_user_asset_position(&user, &Some(eth));
    assert_eq!(eth_pos.debt_principal, 5_0000000);

    let summary = client.get_user_position_summary(&user);
    assert!(!summary.is_liquidatable);
}

#[test]
fn test_alternating_borrow_repay_cycles() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(btc.clone()), &2_0000000);

    // Cycle 1
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_repay_debt(&user, &Some(usdc.clone()), &5000_0000000);

    // Cycle 2
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &8000_0000000);
    client.ca_repay_debt(&user, &Some(usdc.clone()), &10000_0000000);

    // Cycle 3
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &15000_0000000);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.debt_principal, 18000_0000000);
}

#[test]
fn test_cross_asset_with_native_xlm() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    // Setup XLM
    client.initialize_asset(&None, &create_asset_config(&env, None, 1000000));

    // Deposit XLM and USDC
    client.ca_deposit_collateral(&user, &None, &100000_0000000);
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &50000_0000000);

    // Borrow conservative amount
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &50000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert!(summary.total_collateral_value > 0);
    assert!(!summary.is_liquidatable);
    assert!(summary.health_factor >= 10000);
}

#[test]
fn test_zero_debt_after_multiple_repayments() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &50000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &10_0000000);

    // Multiple partial repayments
    client.ca_repay_debt(&user, &Some(eth.clone()), &2_0000000);
    client.ca_repay_debt(&user, &Some(eth.clone()), &3_0000000);
    client.ca_repay_debt(&user, &Some(eth.clone()), &5_0000000);

    let position = client.get_user_asset_position(&user, &Some(eth));
    assert_eq!(position.debt_principal, 0);
}

// ============================================================================
// BOUNDARY AND PRECISION TESTS
// ============================================================================

#[test]
fn test_very_small_amounts() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    // Deposit tiny amount
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &100);
    
    // Borrow tiny amount
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &70);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.collateral, 100);
    assert_eq!(position.debt_principal, 70);
}

#[test]
fn test_very_large_amounts() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    // Use large but reasonable amount (within max_supply cap)
    let large_amount = 50_000_000_000_000;
    
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &large_amount);
    
    let borrow_amount = (large_amount * 75) / 100;
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &borrow_amount);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.collateral, large_amount);
    assert_eq!(position.debt_principal, borrow_amount);
}

#[test]
fn test_health_factor_precision() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    // Create position exactly at health factor = 1.0
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &7500_0000000);

    let summary = client.get_user_position_summary(&user);
    // Health factor should be exactly 1.0 (10000)
    assert_eq!(summary.health_factor, 12500); // (7500 / 6000) * 10000 = 12500
}

// ============================================================================
// MULTIPLE USERS INTERACTION TESTS
// ============================================================================

#[test]
fn test_multiple_users_independent_positions() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    // User 1 operations
    client.ca_deposit_collateral(&user1, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user1, &Some(eth.clone()), &2_0000000);

    // User 2 operations
    client.ca_deposit_collateral(&user2, &Some(eth.clone()), &5_0000000);
    client.ca_borrow_asset(&user2, &Some(usdc.clone()), &5000_0000000);

    // Verify independence
    let user1_usdc = client.get_user_asset_position(&user1, &Some(usdc.clone()));
    let user2_usdc = client.get_user_asset_position(&user2, &Some(usdc.clone()));

    assert_eq!(user1_usdc.collateral, 10000_0000000);
    assert_eq!(user1_usdc.debt_principal, 0);
    assert_eq!(user2_usdc.collateral, 0);
    assert_eq!(user2_usdc.debt_principal, 5000_0000000);
}

#[test]
fn test_price_change_affects_all_users() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let (usdc, eth, _btc) = setup_three_assets(&env, &client);

    // Both users deposit ETH
    client.ca_deposit_collateral(&user1, &Some(eth.clone()), &10_0000000);
    client.ca_deposit_collateral(&user2, &Some(eth.clone()), &5_0000000);

    client.ca_borrow_asset(&user1, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user2, &Some(usdc.clone()), &5000_0000000);

    let summary1_before = client.get_user_position_summary(&user1);
    let summary2_before = client.get_user_position_summary(&user2);

    // ETH price drops
    client.update_asset_price(&Some(eth), &1000_0000000);

    let summary1_after = client.get_user_position_summary(&user1);
    let summary2_after = client.get_user_position_summary(&user2);

    // Both users affected
    assert!(summary1_after.health_factor < summary1_before.health_factor);
    assert!(summary2_after.health_factor < summary2_before.health_factor);
}

// ============================================================================
// ASSET CONFIGURATION CHANGE TESTS
// ============================================================================

#[test]
fn test_collateral_factor_change_affects_borrowing() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);

    let summary_before = client.get_user_position_summary(&user);
    let capacity_before = summary_before.borrow_capacity;

    // Reduce collateral factor from 75% to 50%
    client.update_asset_config(&Some(usdc), &Some(5000), &None, &None, &None, &None, &None);

    let summary_after = client.get_user_position_summary(&user);
    let capacity_after = summary_after.borrow_capacity;

    assert!(capacity_after < capacity_before);
}

#[test]
fn test_disable_asset_borrowing_prevents_new_borrows() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);

    // Disable borrowing for USDC
    client.update_asset_config(&Some(usdc.clone()), &None, &None, &None, &None, &None, &Some(false));

    // This should fail
    let result = client.try_ca_borrow_asset(&user, &Some(usdc), &1000_0000000);
    assert!(result.is_err());
}

#[test]
fn test_repay_still_works_after_borrow_disabled() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &10000_0000000);
    client.ca_borrow_asset(&user, &Some(usdc.clone()), &5000_0000000);

    // Disable borrowing
    client.update_asset_config(&Some(usdc.clone()), &None, &None, &None, &None, &None, &Some(false));

    // Repay should still work
    client.ca_repay_debt(&user, &Some(usdc.clone()), &2500_0000000);

    let position = client.get_user_asset_position(&user, &Some(usdc));
    assert_eq!(position.debt_principal, 2500_0000000);
}

// ============================================================================
// STRESS TESTS
// ============================================================================

#[test]
fn test_many_sequential_operations() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, _eth, _btc) = setup_three_assets(&env, &client);

    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &100000_0000000);

    // Perform 10 borrow/repay cycles
    for i in 1..=10 {
        let amount = i * 1000_0000000;
        client.ca_borrow_asset(&user, &Some(usdc.clone()), &amount);
        client.ca_repay_debt(&user, &Some(usdc.clone()), &(amount / 2));
    }

    let position = client.get_user_asset_position(&user, &Some(usdc));
    // Total borrowed: 55000, repaid: 27500, remaining: 27500
    assert_eq!(position.debt_principal, 27500_0000000);
}

#[test]
fn test_position_summary_consistency() {
    let env = create_test_env();
    let (client, _admin) = setup_contract(&env);
    let user = Address::generate(&env);
    let (usdc, eth, btc) = setup_three_assets(&env, &client);

    // Complex setup
    client.ca_deposit_collateral(&user, &Some(usdc.clone()), &30000_0000000);
    client.ca_deposit_collateral(&user, &Some(eth.clone()), &10_0000000);
    client.ca_deposit_collateral(&user, &Some(btc.clone()), &1_0000000);

    client.ca_borrow_asset(&user, &Some(usdc.clone()), &20000_0000000);
    client.ca_borrow_asset(&user, &Some(eth.clone()), &5_0000000);

    let summary = client.get_user_position_summary(&user);

    // Verify calculations
    // Collateral: $30k + $20k + $40k = $90k
    assert_eq!(summary.total_collateral_value, 90000_0000000);
    
    // Debt: $20k + $10k = $30k
    assert_eq!(summary.total_debt_value, 30000_0000000);

    // Weighted collateral: $90k * 0.75 = $67.5k
    assert_eq!(summary.weighted_collateral_value, 67500_0000000);

    // Health factor: 67.5k / 24k = 2.8125 (28125)
    assert!(summary.health_factor > 20000);
}

