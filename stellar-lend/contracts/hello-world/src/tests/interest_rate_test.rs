//! Interest Rate Model Tests
//!
//! This module contains comprehensive tests for the dynamic interest rate model.
//! It covers:
//! - Utilization-based rate calculations
//! - Rate behavior below and above kink
//! - Rate floor and ceiling enforcement
//! - Emergency rate adjustments
//! - Configuration updates
//! - Edge cases (0%, 100% utilization)
//! - Security and authorization scenarios

use crate::deposit::{DepositDataKey, ProtocolAnalytics};
use crate::interest_rate::{
    calculate_accrued_interest, get_interest_rate_config, InterestRateConfig,
};
use crate::{HelloContract, HelloContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

// =============================================================================
// CONSTANTS
// =============================================================================

const SECONDS_PER_YEAR: u64 = 365 * 86400;

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Creates a test environment with all auths mocked
fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Sets up admin and initializes the contract
fn setup_contract_with_admin(env: &Env) -> (Address, Address, HelloContractClient<'_>) {
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(env, &contract_id);
    let admin = Address::generate(env);

    // Initialize contract with admin
    client.initialize(&admin);

    (contract_id, admin, client)
}

/// Helper to set protocol analytics (deposits and borrows) for utilization testing
fn set_protocol_analytics(
    env: &Env,
    contract_id: &Address,
    total_deposits: i128,
    total_borrows: i128,
) {
    env.as_contract(contract_id, || {
        let analytics_key = DepositDataKey::ProtocolAnalytics;
        let analytics = ProtocolAnalytics {
            total_deposits,
            total_borrows,
            total_value_locked: total_deposits,
        };
        env.storage().persistent().set(&analytics_key, &analytics);
    });
}

/// Helper to get interest rate config from storage
fn get_config(env: &Env, contract_id: &Address) -> Option<InterestRateConfig> {
    env.as_contract(contract_id, || get_interest_rate_config(env))
}

// =============================================================================
// UTILIZATION CALCULATION TESTS
// =============================================================================

/// Test utilization at 0% (no borrows)
#[test]
fn test_utilization_zero_borrows() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set deposits but no borrows
    set_protocol_analytics(&env, &contract_id, 10000, 0);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 0);
}

/// Test utilization at 50%
#[test]
fn test_utilization_fifty_percent() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set 50% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 5000);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 5000); // 50% = 5000 basis points
}

/// Test utilization at 80% (kink)
#[test]
fn test_utilization_at_kink() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set 80% utilization (default kink)
    set_protocol_analytics(&env, &contract_id, 10000, 8000);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 8000); // 80% = 8000 basis points
}

/// Test utilization at 100%
#[test]
fn test_utilization_full() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set 100% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 10000);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 10000); // 100% = 10000 basis points
}

/// Test utilization caps at 100% even with more borrows than deposits
#[test]
fn test_utilization_capped_at_100() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set more borrows than deposits (shouldn't happen normally)
    set_protocol_analytics(&env, &contract_id, 10000, 15000);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 10000); // Capped at 100%
}

/// Test utilization with no deposits returns 0
#[test]
fn test_utilization_no_deposits() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // No deposits
    set_protocol_analytics(&env, &contract_id, 0, 0);

    let utilization = client.get_utilization();
    assert_eq!(utilization, 0);
}

// =============================================================================
// BORROW RATE CALCULATION TESTS
// =============================================================================

/// Test borrow rate at 0% utilization
#[test]
fn test_borrow_rate_at_zero_utilization() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 0);

    let borrow_rate = client.get_borrow_rate();

    // At 0% utilization, rate should be base rate (100 bps = 1%)
    // But it should be at least the floor (50 bps)
    assert!(borrow_rate >= 50);
    assert_eq!(borrow_rate, 100); // Base rate
}

/// Test borrow rate below kink (linear increase)
#[test]
fn test_borrow_rate_below_kink() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // 40% utilization (below 80% kink)
    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    let borrow_rate = client.get_borrow_rate();

    // Rate = base_rate + (utilization / kink_utilization) * multiplier
    // Rate = 100 + (4000 / 8000) * 2000 = 100 + 1000 = 1100 bps (11%)
    assert_eq!(borrow_rate, 1100);
}

/// Test borrow rate at kink
#[test]
fn test_borrow_rate_at_kink() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Exactly at kink (80%)
    set_protocol_analytics(&env, &contract_id, 10000, 8000);

    let borrow_rate = client.get_borrow_rate();

    // Rate at kink = base_rate + multiplier = 100 + 2000 = 2100 bps (21%)
    assert_eq!(borrow_rate, 2100);
}

/// Test borrow rate above kink (steeper increase)
#[test]
fn test_borrow_rate_above_kink() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // 90% utilization (above 80% kink)
    set_protocol_analytics(&env, &contract_id, 10000, 9000);

    let borrow_rate = client.get_borrow_rate();

    // Rate = rate_at_kink + (utilization - kink) / (10000 - kink) * jump_multiplier
    // Rate = 2100 + (9000 - 8000) / (10000 - 8000) * 10000
    // Rate = 2100 + (1000 / 2000) * 10000 = 2100 + 5000 = 7100 bps (71%)
    assert_eq!(borrow_rate, 7100);
}

/// Test borrow rate at 100% utilization
#[test]
fn test_borrow_rate_at_full_utilization() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // 100% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 10000);

    let borrow_rate = client.get_borrow_rate();

    // Rate = rate_at_kink + jump_multiplier = 2100 + 10000 = 12100
    // But capped at ceiling (10000 bps = 100%)
    assert_eq!(borrow_rate, 10000);
}

// =============================================================================
// SUPPLY RATE CALCULATION TESTS
// =============================================================================

/// Test supply rate calculation
#[test]
fn test_supply_rate_calculation() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // 40% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    let borrow_rate = client.get_borrow_rate();
    let supply_rate = client.get_supply_rate();

    // Supply rate = borrow_rate - spread (200 bps = 2%)
    // Expected: 1100 - 200 = 900 bps (9%)
    assert_eq!(supply_rate, borrow_rate - 200);
    assert_eq!(supply_rate, 900);
}

/// Test supply rate at low utilization respects floor
#[test]
fn test_supply_rate_floor_enforcement() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Very low utilization
    set_protocol_analytics(&env, &contract_id, 10000, 100);

    let supply_rate = client.get_supply_rate();

    // Supply rate should not go below floor (50 bps)
    assert!(supply_rate >= 50);
}

// =============================================================================
// RATE FLOOR AND CEILING TESTS
// =============================================================================

/// Test rate floor is enforced
#[test]
fn test_rate_floor_enforcement() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    // Set protocol analytics for very low utilization
    set_protocol_analytics(&env, &contract_id, 10000, 0);

    // Set a custom config with higher floor
    client.update_interest_rate_config(
        &admin,
        &Some(10), // Very low base rate (0.1%)
        &None,
        &None,
        &None,
        &Some(100), // Floor: 1%
        &None,
        &None,
    );

    let borrow_rate = client.get_borrow_rate();

    // Rate should be at floor even if calculated rate is lower
    assert!(borrow_rate >= 100);
}

/// Test rate ceiling is enforced
#[test]
fn test_rate_ceiling_enforcement() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Set 100% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 10000);

    let borrow_rate = client.get_borrow_rate();

    // Rate should not exceed ceiling (10000 bps = 100%)
    assert!(borrow_rate <= 10000);
    assert_eq!(borrow_rate, 10000);
}

// =============================================================================
// EMERGENCY RATE ADJUSTMENT TESTS
// =============================================================================

/// Test positive emergency rate adjustment
#[test]
fn test_emergency_rate_adjustment_positive() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    // Get rate before adjustment
    let rate_before = client.get_borrow_rate();

    // Apply positive emergency adjustment (500 bps = 5%)
    client.set_emergency_rate_adjustment(&admin, &500);

    // Get rate after adjustment
    let rate_after = client.get_borrow_rate();

    // Rate should increase by 500 bps
    assert_eq!(rate_after, rate_before + 500);
}

/// Test negative emergency rate adjustment
#[test]
fn test_emergency_rate_adjustment_negative() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    // Get rate before adjustment
    let rate_before = client.get_borrow_rate();

    // Apply negative emergency adjustment (-300 bps = -3%)
    client.set_emergency_rate_adjustment(&admin, &(-300));

    // Get rate after adjustment
    let rate_after = client.get_borrow_rate();

    // Rate should decrease by 300 bps
    assert_eq!(rate_after, rate_before - 300);
}

/// Test emergency adjustment unauthorized
#[test]
#[should_panic(expected = "HostError")]
fn test_emergency_rate_adjustment_unauthorized() {
    let env = create_test_env();
    let (_contract_id, _admin, client) = setup_contract_with_admin(&env);
    let unauthorized = Address::generate(&env);

    client.set_emergency_rate_adjustment(&unauthorized, &500);
}

/// Test emergency adjustment exceeds bounds
#[test]
#[should_panic(expected = "HostError")]
fn test_emergency_rate_adjustment_exceeds_bounds() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    // Try to set adjustment > 100% (10000 bps)
    client.set_emergency_rate_adjustment(&admin, &15000);
}

// =============================================================================
// CONFIGURATION UPDATE TESTS
// =============================================================================

/// Test updating base rate
#[test]
fn test_update_config_base_rate() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 0);

    // Update base rate to 200 bps (2%)
    client.update_interest_rate_config(
        &admin,
        &Some(200), // new base rate
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
    );

    let borrow_rate = client.get_borrow_rate();
    assert_eq!(borrow_rate, 200);
}

/// Test updating kink utilization
#[test]
fn test_update_config_kink() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    // Set 70% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 7000);

    // Change kink from 80% to 60%
    client.update_interest_rate_config(
        &admin,
        &None,
        &Some(6000), // new kink: 60%
        &None,
        &None,
        &None,
        &None,
        &None,
    );

    // Now 70% is above kink, so rate should be higher
    let borrow_rate = client.get_borrow_rate();

    // With kink at 60%, rate at kink = 100 + 2000 = 2100
    // Above kink: 2100 + (7000 - 6000) / (10000 - 6000) * 10000 = 2100 + 2500 = 4600
    assert_eq!(borrow_rate, 4600);
}

/// Test updating multiplier
#[test]
fn test_update_config_multiplier() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    // Set 40% utilization
    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    // Change multiplier from 2000 to 4000 (40%)
    client.update_interest_rate_config(
        &admin,
        &None,
        &None,
        &Some(4000), // new multiplier
        &None,
        &None,
        &None,
        &None,
    );

    let borrow_rate = client.get_borrow_rate();

    // Rate = 100 + (4000 / 8000) * 4000 = 100 + 2000 = 2100
    assert_eq!(borrow_rate, 2100);
}

/// Test updating jump multiplier
#[test]
fn test_update_config_jump_multiplier() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    // Set 90% utilization (above kink)
    set_protocol_analytics(&env, &contract_id, 10000, 9000);

    // Change jump multiplier from 10000 to 5000 (50%)
    client.update_interest_rate_config(
        &admin,
        &None,
        &None,
        &None,
        &Some(5000), // new jump multiplier
        &None,
        &None,
        &None,
    );

    let borrow_rate = client.get_borrow_rate();

    // Rate = 2100 + (1000 / 2000) * 5000 = 2100 + 2500 = 4600
    assert_eq!(borrow_rate, 4600);
}

/// Test updating spread
#[test]
fn test_update_config_spread() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 4000);

    // Change spread from 200 to 500 (5%)
    client.update_interest_rate_config(
        &admin,
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
        &Some(500), // new spread
    );

    let borrow_rate = client.get_borrow_rate();
    let supply_rate = client.get_supply_rate();

    // Supply rate should now be borrow rate - 500
    assert_eq!(supply_rate, borrow_rate - 500);
}

/// Test config update unauthorized
#[test]
#[should_panic(expected = "HostError")]
fn test_update_config_unauthorized() {
    let env = create_test_env();
    let (_contract_id, _admin, client) = setup_contract_with_admin(&env);
    let unauthorized = Address::generate(&env);

    client.update_interest_rate_config(
        &unauthorized,
        &Some(200),
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

// =============================================================================
// INVALID PARAMETER TESTS
// =============================================================================

/// Test negative base rate is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_base_rate_negative() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &Some(-100), // Invalid: negative
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

/// Test base rate > 100% is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_base_rate_too_high() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &Some(15000), // Invalid: > 100%
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

/// Test kink = 0 is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_kink_zero() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &None,
        &Some(0), // Invalid: kink must be > 0
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

/// Test kink = 100% is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_kink_100_percent() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &None,
        &Some(10000), // Invalid: kink must be < 100%
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

/// Test negative multiplier is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_multiplier_negative() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &None,
        &None,
        &Some(-100), // Invalid: negative
        &None,
        &None,
        &None,
        &None,
    );
}

/// Test floor > ceiling is rejected
#[test]
#[should_panic(expected = "HostError")]
fn test_invalid_floor_above_ceiling() {
    let env = create_test_env();
    let (_contract_id, admin, client) = setup_contract_with_admin(&env);

    client.update_interest_rate_config(
        &admin,
        &None,
        &None,
        &None,
        &None,
        &Some(5000), // Floor: 50%
        &Some(3000), // Ceiling: 30% - Invalid: < floor
        &None,
    );
}

// =============================================================================
// ACCRUED INTEREST CALCULATION TESTS
// =============================================================================

/// Test accrued interest calculation
#[test]
fn test_accrued_interest_calculation() {
    // Principal: 1,000,000 (1M units)
    // Rate: 1000 bps (10% annual)
    // Time: 1 year
    let principal = 1_000_000i128;
    let rate_bps = 1000i128;
    let last_accrual = 0u64;
    let current_time = SECONDS_PER_YEAR;

    let interest =
        calculate_accrued_interest(principal, last_accrual, current_time, rate_bps).unwrap();

    // Expected: 1,000,000 * 10% = 100,000
    assert_eq!(interest, 100_000);
}

/// Test accrued interest for partial year
#[test]
fn test_accrued_interest_partial_year() {
    // Principal: 1,000,000
    // Rate: 1000 bps (10% annual)
    // Time: 6 months (half year)
    let principal = 1_000_000i128;
    let rate_bps = 1000i128;
    let last_accrual = 0u64;
    let current_time = SECONDS_PER_YEAR / 2;

    let interest =
        calculate_accrued_interest(principal, last_accrual, current_time, rate_bps).unwrap();

    // Expected: 1,000,000 * 10% * 0.5 = 50,000
    assert_eq!(interest, 50_000);
}

/// Test accrued interest with zero principal
#[test]
fn test_accrued_interest_zero_principal() {
    let interest = calculate_accrued_interest(0, 0, SECONDS_PER_YEAR, 1000).unwrap();
    assert_eq!(interest, 0);
}

/// Test accrued interest with zero time elapsed
#[test]
fn test_accrued_interest_zero_time() {
    let interest = calculate_accrued_interest(1_000_000, 1000, 1000, 1000).unwrap();
    assert_eq!(interest, 0);
}

/// Test accrued interest with time going backwards (edge case)
#[test]
fn test_accrued_interest_time_backwards() {
    let interest = calculate_accrued_interest(1_000_000, 2000, 1000, 1000).unwrap();
    assert_eq!(interest, 0);
}

/// Test accrued interest with extreme values triggering overflow
#[test]
fn test_accrued_interest_extreme_overflow() {
    // Principal: i128::MAX
    // Rate: 10000 bps (100% annual)
    // Time: 100 years
    let principal = i128::MAX;
    let rate_bps = 10000;
    let last_accrual = 0;
    let current_time = 100 * SECONDS_PER_YEAR;

    let result = calculate_accrued_interest(principal, last_accrual, current_time, rate_bps);

    // Should return Overflow error instead of panicking
    assert!(result.is_err());
}

// =============================================================================
// RATE TRANSITION TESTS
// =============================================================================

/// Test rate changes smoothly across utilization levels
#[test]
fn test_rate_changes_with_utilization() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    let mut previous_rate = 0i128;

    // Test rates at various utilization levels
    for util in (0..=100).step_by(10) {
        let util_bps = (util * 100) as i128;
        set_protocol_analytics(&env, &contract_id, 10000, util_bps);

        let rate = client.get_borrow_rate();

        // Rate should always increase with utilization
        assert!(
            rate >= previous_rate,
            "Rate decreased at {}% utilization",
            util
        );
        previous_rate = rate;
    }
}

/// Test rate jump at kink
#[test]
fn test_rate_jump_at_kink() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Just below kink (79%)
    set_protocol_analytics(&env, &contract_id, 10000, 7900);
    let rate_below_kink = client.get_borrow_rate();

    // Just above kink (81%)
    set_protocol_analytics(&env, &contract_id, 10000, 8100);
    let rate_above_kink = client.get_borrow_rate();

    // Rate above kink should be noticeably higher due to jump multiplier
    assert!(rate_above_kink > rate_below_kink);

    // The jump should be significant (more than just linear increase)
    let linear_increase = (rate_below_kink * 200) / 7900; // Rough linear increase for 2%
    let actual_increase = rate_above_kink - rate_below_kink;
    assert!(
        actual_increase > linear_increase,
        "Jump multiplier not taking effect"
    );
}

// =============================================================================
// EDGE CASE TESTS
// =============================================================================

/// Test very small utilization
#[test]
fn test_very_small_utilization() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // 0.01% utilization
    set_protocol_analytics(&env, &contract_id, 1_000_000, 100);

    let rate = client.get_borrow_rate();

    // Rate should be close to base rate
    assert!(rate >= 50); // At least floor
    assert!(rate <= 200); // Not too far from base rate
}

/// Test very large values
#[test]
fn test_large_values() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    // Large deposits and borrows
    set_protocol_analytics(
        &env,
        &contract_id,
        1_000_000_000_000i128,
        500_000_000_000i128,
    );

    let utilization = client.get_utilization();
    let rate = client.get_borrow_rate();

    // 50% utilization
    assert_eq!(utilization, 5000);
    // Rate should be calculated correctly
    assert!(rate > 0);
}

/// Test rate consistency across multiple calls
#[test]
fn test_rate_consistency() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);

    set_protocol_analytics(&env, &contract_id, 10000, 5000);

    // Multiple calls should return same rate
    let rate1 = client.get_borrow_rate();
    let rate2 = client.get_borrow_rate();
    let rate3 = client.get_borrow_rate();

    assert_eq!(rate1, rate2);
    assert_eq!(rate2, rate3);
}

// =============================================================================
// INTEGRATION TESTS
// =============================================================================

/// Test full interest rate workflow
#[test]
fn test_full_interest_rate_workflow() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract_with_admin(&env);

    // 1. Check initial config
    let config = get_config(&env, &contract_id).unwrap();
    assert_eq!(config.base_rate_bps, 100);
    assert_eq!(config.kink_utilization_bps, 8000);

    // 2. Set initial utilization
    set_protocol_analytics(&env, &contract_id, 10000, 4000);
    let initial_rate = client.get_borrow_rate();
    assert_eq!(initial_rate, 1100); // base + half of multiplier

    // 3. Update config
    client.update_interest_rate_config(
        &admin,
        &Some(200),
        &None,
        &None,
        &None,
        &None,
        &None,
        &None,
    );

    // 4. Verify rate changed
    let new_rate = client.get_borrow_rate();
    assert!(new_rate > initial_rate);

    // 5. Apply emergency adjustment
    client.set_emergency_rate_adjustment(&admin, &300);

    // 6. Verify emergency adjustment applied
    let emergency_rate = client.get_borrow_rate();
    assert_eq!(emergency_rate, new_rate + 300);
}

/// Test interest accrual over time
#[test]
fn test_interest_accrual_over_time() {
    let env = create_test_env();
    let (contract_id, _admin, client) = setup_contract_with_admin(&env);
    let user = Address::generate(&env);

    // Initial deposit
    client.deposit_collateral(&user, &None, &100_000);

    // Simulate borrowing
    env.as_contract(&contract_id, || {
        let analytics_key = DepositDataKey::ProtocolAnalytics;
        let analytics = ProtocolAnalytics {
            total_deposits: 100_000,
            total_borrows: 50_000,
            total_value_locked: 100_000,
        };
        env.storage().persistent().set(&analytics_key, &analytics);
    });

    // Get rate
    let rate = client.get_borrow_rate();

    // Calculate expected interest for 1 year on 50,000 borrowed
    let expected_interest = calculate_accrued_interest(50_000, 0, SECONDS_PER_YEAR, rate).unwrap();

    // Interest should be reasonable (between 1% and 100% of principal)
    assert!(expected_interest > 500); // > 1%
    assert!(expected_interest < 50_000); // < 100%
}
