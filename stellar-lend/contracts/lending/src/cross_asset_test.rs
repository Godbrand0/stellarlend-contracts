//! Cross-asset lending module tests.
//!
//! Coverage:
//! - Admin initialisation
//! - Asset parameter configuration (valid and invalid)
//! - Deposit / borrow / repay operations
//! - Basic health-factor checks
//!
//! NOTE: `withdraw_asset` is not tested here because it performs a real token
//! transfer that requires a deployed token contract. Those scenarios live in
//! integration tests that set up a mock token.

use crate::cross_asset::{AssetParams, CrossAssetError};
use crate::{LendingContract, LendingContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

// ─────────────────────────────────────────────────────────────────────────────
// Constants that mirror cross_asset.rs internals
// ─────────────────────────────────────────────────────────────────────────────

/// BPS_SCALE = 10_000; health factor ≥ this value is considered healthy.
const HF_HEALTHY: i128 = 10_000;
/// Sentinel health factor when the position carries no debt.
const HF_NO_DEBT: i128 = 1_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (LendingContractClient<'_>, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_admin(&admin);
    (client, admin)
}

/// Build an `AssetParams` with the given LTV. Liquidation threshold = LTV + 500 bps (capped at 10 000).
fn asset_params(env: &Env, ltv: i128) -> AssetParams {
    let threshold = (ltv + 500).min(10_000);
    AssetParams {
        ltv,
        liquidation_threshold: threshold,
        price_feed: Address::generate(env),
        debt_ceiling: 1_000_000_000_000,
        is_active: true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Admin initialisation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_initialize_admin_stores_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);
    // Verify that admin is stored by confirming we can call a protected function.
    let asset = Address::generate(&env);
    let params = asset_params(&env, 7500);
    // If admin were not set, set_asset_params would return Unauthorized.
    client.set_asset_params(&asset, &params);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Asset parameter configuration
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_set_asset_params_success() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    let params = asset_params(&env, 7500);
    client.set_asset_params(&asset, &params);
}

#[test]
fn test_set_asset_params_stores_and_allows_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &1_000);
}

#[test]
fn test_deposit_on_inactive_asset_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    let params = AssetParams {
        ltv: 7500,
        liquidation_threshold: 8000,
        price_feed: Address::generate(&env),
        debt_ceiling: 1_000_000_000,
        is_active: false, // disabled
    };
    client.set_asset_params(&asset, &params);

    let user = Address::generate(&env);
    let result = client.try_deposit_collateral_asset(&user, &asset, &1_000);
    assert_eq!(result, Err(Ok(CrossAssetError::AssetNotSupported)));
}

#[test]
fn test_deposit_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    let result = client.try_deposit_collateral_asset(&user, &asset, &0);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

#[test]
fn test_deposit_negative_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    let result = client.try_deposit_collateral_asset(&user, &asset, &-100);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Borrow operations
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_borrow_on_unknown_asset_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    let user = Address::generate(&env);
    let result = client.try_borrow_asset(&user, &asset, &100);
    assert_eq!(result, Err(Ok(CrossAssetError::AssetNotSupported)));
}

#[test]
fn test_borrow_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    let result = client.try_borrow_asset(&user, &asset, &0);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

#[test]
fn test_borrow_without_collateral_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    let result = client.try_borrow_asset(&user, &asset, &500);
    assert_eq!(result, Err(Ok(CrossAssetError::InsufficientCollateral)));
}

#[test]
fn test_borrow_exceeds_health_factor_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    // LTV 7500 → max borrow = collateral * 0.75
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);

    // Borrow more than weighted collateral allows (> 7500)
    let result = client.try_borrow_asset(&user, &asset, &8_000);
    assert_eq!(result, Err(Ok(CrossAssetError::InsufficientCollateral)));
}

#[test]
fn test_borrow_at_exact_capacity_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    // LTV 7500 → max borrow for 10_000 collateral = 7500
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &7_500);
}

#[test]
fn test_borrow_exceeds_debt_ceiling_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    let params = AssetParams {
        ltv: 9000,
        liquidation_threshold: 9500,
        price_feed: Address::generate(&env),
        debt_ceiling: 100, // very small ceiling
        is_active: true,
    };
    client.set_asset_params(&asset, &params);

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);

    let result = client.try_borrow_asset(&user, &asset, &101);
    assert_eq!(result, Err(Ok(CrossAssetError::DebtCeilingReached)));
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Repay operations
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_repay_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    let result = client.try_repay_asset(&user, &asset, &0);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

#[test]
fn test_repay_reduces_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);

    // Repay 2000; debt should fall to 3000.
    client.repay_asset(&user, &asset, &2_000);

    let summary = client.get_cross_position_summary(&user);
    // debt_value = 3000 (price = $1 scaled)
    assert_eq!(summary.total_debt_usd, 3_000);
}

#[test]
fn test_repay_overpay_capped_at_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);

    // Repay more than the outstanding debt — should be capped at 5000.
    client.repay_asset(&user, &asset, &999_999);

    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_debt_usd, 0);
    assert_eq!(summary.health_factor, HF_NO_DEBT);
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Position summary – basic invariants
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_summary_empty_position_has_zero_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    let summary = client.get_cross_position_summary(&user);

    assert_eq!(summary.total_collateral_usd, 0);
    assert_eq!(summary.total_debt_usd, 0);
    assert_eq!(summary.health_factor, HF_NO_DEBT);
}

#[test]
fn test_summary_collateral_only_has_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    client.set_asset_params(&asset, &asset_params(&env, 7500));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);

    let summary = client.get_cross_position_summary(&user);
    // No debt → sentinel health factor
    assert_eq!(summary.health_factor, HF_NO_DEBT);
    // Collateral value: 10_000 * 10_000_000 / 10_000_000 = 10_000
    assert_eq!(summary.total_collateral_usd, 10_000);
    assert_eq!(summary.total_debt_usd, 0);
}

#[test]
fn test_summary_debt_only_position_reflects_uncollateralised_state() {
    // A user can end up with debt > 0 but collateral = 0 only through
    // a test shortcut (direct state manipulation). The summary must still
    // be internally consistent: if collateral_usd == 0 the health factor
    // formula yields 0 (weighted_collateral=0 → 0 * scale / debt = 0).
    // We verify the formula works with very small collateral instead.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = Address::generate(&env);
    // LTV = 10_000 (100%) to maximise borrow capacity
    client.set_asset_params(&asset, &asset_params(&env, 10_000));

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &1);
    client.borrow_asset(&user, &asset, &1);

    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_collateral_usd, 1);
    assert_eq!(summary.total_debt_usd, 1);
    // weighted = 1 * 10_000 / 10_000 = 1; HF = 1 * 10_000 / 1 = 10_000
    assert_eq!(summary.health_factor, HF_HEALTHY);
}
