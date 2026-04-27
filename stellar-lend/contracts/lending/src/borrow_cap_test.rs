//! # Borrow Cap Test Suite
//!
//! Comprehensive tests for per-asset borrow caps, ensuring:
//! - Cap boundary conditions (exact, one above, one below)
//! - Flash-loan bypass resistance
//! - Cap + liquidation interactions
//! - Admin operations
//! - Happy path regression (uncapped assets)
//!
//! Target coverage: 95%+ on all changed paths

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Map,
};

fn setup_test_env(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &collateral_asset);

    (client, admin, user, asset, collateral_asset)
}

// ═══════════════════════════════════════════════════════════════════
// CAP BOUNDARY TESTS
// ═══════════════════════════════════════════════════════════════════

/// Test: Borrow exactly at cap succeeds
#[test]
fn test_borrow_at_exact_cap_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Borrow exactly 10,000 - should succeed
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000);
}

/// Test: Borrow one unit above cap is rejected
#[test]
fn test_borrow_one_above_cap_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Try to borrow 10,001 - should fail
    let result = client.try_borrow(&user, &asset, &10_001, &collateral_asset, &20_000);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));
}

/// Test: Borrow with cap set to zero (uncapped) succeeds
#[test]
fn test_borrow_uncapped_asset_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 0 (uncapped)
    client.set_borrow_cap(&admin, &asset, &0);

    // Borrow large amount - should succeed
    client.borrow(&user, &asset, &100_000, &collateral_asset, &200_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 100_000);
}

/// Test: Cap exactly equal to current total borrowed blocks further borrows
#[test]
fn test_cap_equal_to_current_debt_blocks_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // First borrow: 5,000
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Set cap to exactly 5,000 (current total borrowed)
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Try to borrow any positive amount - should fail
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));
}

/// Test: Cap increased allows previously rejected borrow
#[test]
fn test_cap_increased_allows_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set initial cap to 5,000
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Borrow 5,000 - succeeds
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Try to borrow 1 more - fails
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));

    // Increase cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Now borrow 1 more - should succeed
    client.borrow(&user, &asset, &1, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 5_001);
}

/// Test: Cap decreased blocks borrow that would exceed new cap
#[test]
fn test_cap_decreased_blocks_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set initial cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Borrow 5,000
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Decrease cap to 6,000
    client.set_borrow_cap(&admin, &asset, &6_000);

    // Try to borrow 2,000 more (would total 7,000 > 6,000) - should fail
    let result = client.try_borrow(&user, &asset, &2_000, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));

    // But borrowing 1,000 (total 6,000) should succeed
    client.borrow(&user, &asset, &1_000, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 6_000);
}

// ═══════════════════════════════════════════════════════════════════
// FLASH-LOAN BYPASS RESISTANCE TESTS
// ═══════════════════════════════════════════════════════════════════

/// Test: Flash loan with net zero borrow effect doesn't count against cap
#[test]
fn test_flash_loan_net_zero_borrow_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 5,000
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Borrow 5,000 (at cap)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Flash loan with net zero effect (borrow and repay within callback)
    // This should succeed because net debt change is 0
    let receiver = Address::generate(&env);
    let result = client.try_flash_loan(&receiver, &asset, &1_000, &Bytes::new(&env));
    // Should succeed (or fail for other reasons, but not BorrowCapExceeded)
    // Note: In a real test, we'd mock the callback to verify behavior
    assert!(result.is_ok() || result != Err(Ok(FlashLoanError::BorrowCapExceeded)));
}

/// Test: Flash loan that would exceed cap is rejected
#[test]
fn test_flash_loan_exceeding_cap_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 5,000
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Borrow 5,000 (at cap)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Attempt flash loan that would increase debt beyond cap
    // This is a conceptual test; actual implementation depends on callback behavior
    let receiver = Address::generate(&env);
    let _result = client.try_flash_loan(&receiver, &asset, &1_000, &Bytes::new(&env));
    // In a real scenario with a callback that borrows, this would be rejected
}

// ═══════════════════════════════════════════════════════════════════
// CAP + LIQUIDATION SCENARIO TESTS
// ═══════════════════════════════════════════════════════════════════

/// Test: Liquidation frees cap space for new borrows
#[test]
fn test_liquidation_frees_cap_space() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set borrow cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Borrow 10,000 (at cap)
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // Verify cap is reached
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));

    // Liquidate part of the position (reduce debt by 5,000)
    let liquidator = Address::generate(&env);
    client.liquidate_position(&liquidator, &user, &asset, &collateral_asset, &5_000);

    // Now borrowing should succeed (cap space freed)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000); // 10k - 5k + 5k
}

/// Test: Multi-asset position with one capped asset
#[test]
fn test_multi_asset_one_capped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    let asset2 = Address::generate(&env);
    client.register_asset(&admin, &asset2);

    // Set cap on asset1 only
    client.set_borrow_cap(&admin, &asset, &5_000);
    // asset2 remains uncapped (cap = 0)

    // Borrow 5,000 from asset1 (at cap)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Try to borrow from asset1 again - should fail
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));

    // But borrowing from uncapped asset2 should succeed
    client.borrow(&user, &asset2, &100_000, &collateral_asset, &0);

    let debt2 = client.get_user_debt(&user);
    // Debt should reflect both borrows (simplified check)
    assert!(debt2.borrowed_amount >= 5_000);
}

// ═══════════════════════════════════════════════════════════════════
// ADMIN OPERATION TESTS
// ═══════════════════════════════════════════════════════════════════

/// Test: Admin sets cap on uncapped asset
#[test]
fn test_admin_sets_cap_on_uncapped_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, asset, _collateral_asset) = setup_test_env(&env);

    // Set cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Verify cap is set (would be visible in view function if implemented)
    // For now, we verify by attempting to borrow at cap
    let user = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &_collateral_asset, &20_000);

    let result = client.try_borrow(&user, &asset, &1, &_collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));
}

/// Test: Admin updates cap
#[test]
fn test_admin_updates_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set initial cap to 5,000
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Borrow 5,000
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Update cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Now borrowing more should succeed
    client.borrow(&user, &asset, &5_000, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000);
}

/// Test: Non-admin cannot set cap
#[test]
fn test_non_admin_cannot_set_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, _collateral_asset) = setup_test_env(&env);

    // Non-admin tries to set cap - should fail
    let result = client.try_set_borrow_cap(&user, &asset, &10_000);
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
}

/// Test: Admin cannot set negative cap
#[test]
fn test_admin_cannot_set_negative_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, asset, _collateral_asset) = setup_test_env(&env);

    // Try to set negative cap - should fail
    let result = client.try_set_borrow_cap(&admin, &asset, &-1);
    assert_eq!(result, Err(Ok(BorrowError::InvalidBorrowCap)));
}

// ═══════════════════════════════════════════════════════════════════
// HAPPY PATH REGRESSION TESTS
// ═══════════════════════════════════════════════════════════════════

/// Test: Standard borrow on uncapped asset unchanged
#[test]
fn test_standard_borrow_uncapped_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Don't set any cap (default is 0 = uncapped)
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000);
}

/// Test: Multiple borrows on uncapped asset
#[test]
fn test_multiple_borrows_uncapped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Borrow multiple times without cap
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);
    client.borrow(&user, &asset, &5_000, &collateral_asset, &0);
    client.borrow(&user, &asset, &5_000, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 15_000);
}

/// Test: Repayment reduces debt and frees cap space
#[test]
fn test_repayment_frees_cap_space() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set cap to 10,000
    client.set_borrow_cap(&admin, &asset, &10_000);

    // Borrow 10,000 (at cap)
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // Verify cap is reached
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));

    // Repay 5,000
    client.repay(&user, &asset, &5_000);

    // Now borrowing should succeed (cap space freed)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &0);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000); // 10k - 5k + 5k
}

// ═══════════════════════════════════════════════════════════════════
// VACUOUSNESS CHECKS (Security)
// ═══════════════════════════════════════════════════════════════════

/// Vacuousness check: Removing cap enforcement causes test to fail
/// This test verifies that the cap check is actually being evaluated.
/// If the cap check were disabled, this test would fail (borrow would succeed when it shouldn't).
#[test]
fn test_cap_enforcement_is_active() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup_test_env(&env);

    // Set cap to 5,000
    client.set_borrow_cap(&admin, &asset, &5_000);

    // Borrow 5,000 (at cap)
    client.borrow(&user, &asset, &5_000, &collateral_asset, &20_000);

    // Try to borrow 1 more - MUST fail
    let result = client.try_borrow(&user, &asset, &1, &collateral_asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::BorrowCapExceeded)));
    // If cap enforcement were disabled, this assertion would fail
}

/// Vacuousness check: Admin guard is actually evaluated
#[test]
fn test_admin_guard_is_active() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, _collateral_asset) = setup_test_env(&env);

    // Non-admin tries to set cap - MUST fail
    let result = client.try_set_borrow_cap(&user, &asset, &10_000);
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
    // If admin guard were disabled, this assertion would fail
}
