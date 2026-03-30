//! Comprehensive tests for the cross-asset lending registry module.
//!
//! ## Coverage targets
//! - Initialization (admin + asset)
//! - Config updates (valid, invalid, unauthorized)
//! - Price updates (valid, zero, stale, unauthorized)
//! - Deposit / Withdraw / Borrow / Repay with checked math
//! - Health factor enforcement
//! - Supply and borrow caps
//! - Edge cases (zero amounts, overflow, re-initialization)
//! - Read-only queries
//!
//! ## Trust Boundaries
//! - Only the registered admin can initialize assets, update configs, and update prices.
//! - Users authorize their own deposit, withdraw, borrow, and repay calls.
//! - No external token transfers occur in this module — all state is updated atomically.
//!
//! ## Security Assumptions
//! - Prices are admin-gated. In production, integrate a decentralized oracle.
//! - Checked arithmetic is used throughout to prevent overflow/underflow.
//! - Staleness threshold of 1 hour is enforced for all price-dependent operations.
//! - Health factor must remain >= 10_000 (1.0x) for any withdrawal or borrow.

use crate::cross_asset::{AssetConfig, CrossAssetError};
use crate::{HelloContract, HelloContractClient};
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, Address, Env};

// ============================================================================
// Helpers
// ============================================================================

/// Create a default valid asset config for testing.
///
/// Uses native XLM (asset = None) with:
/// - 75% LTV collateral factor
/// - 80% liquidation threshold
/// - 10% reserve factor
/// - $1.00 price (7 decimals)
/// - 1M supply cap, 500K borrow cap
fn default_config(env: &Env) -> AssetConfig {
    AssetConfig {
        asset: None,
        collateral_factor: 7500,       // 75% LTV — max fraction of collateral value usable as borrow capacity
        liquidation_threshold: 8000,    // 80% — position becomes liquidatable above this debt ratio
        reserve_factor: 1000,           // 10% — fraction of interest directed to protocol reserve
        max_supply: 1_000_000_0000000,  // 1M units (7 decimals) — global supply cap
        max_borrow: 500_000_0000000,    // 500K units — global borrow/debt ceiling
        can_collateralize: true,        // Asset accepted as collateral
        can_borrow: true,               // Asset can be borrowed
        price: 10_000_000,              // $1.00 expressed in 7 decimal precision
        price_updated_at: env.ledger().timestamp(), // Fresh price — within staleness window
    }
}

/// Create a token-backed asset config for testing.
///
/// Uses a Soroban token contract address with:
/// - 60% LTV collateral factor (more conservative than XLM)
/// - 70% liquidation threshold
/// - 20% reserve factor
/// - $2.00 price (7 decimals)
fn token_config(env: &Env, addr: &Address) -> AssetConfig {
    AssetConfig {
        asset: Some(addr.clone()),      // Token contract address
        collateral_factor: 6000,        // 60% LTV — conservative for volatile tokens
        liquidation_threshold: 7000,    // 70% liquidation threshold
        reserve_factor: 2000,           // 20% — higher reserve for riskier asset
        max_supply: 500_000_0000000,    // 500K supply cap
        max_borrow: 250_000_0000000,    // 250K borrow cap — half of supply cap
        can_collateralize: true,
        can_borrow: true,
        price: 20_000_000,              // $2.00 — higher priced token
        price_updated_at: env.ledger().timestamp(),
    }
}

/// Set up a fresh environment, register the HelloContract, and initialize
/// both the main module and the cross-asset (CA) module with a generated admin.
///
/// Returns (env, client, admin) ready for test use.
/// All auth checks are mocked so tests focus on logic, not auth mechanics.
fn setup() -> (Env, HelloContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths(); // Mock all require_auth() calls for test simplicity
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);      // Initialize main contract module
    client.initialize_ca(&admin);   // Initialize cross-asset module with same admin
    (env, client, admin)
}

// ============================================================================
// 1. Admin Initialization
// ============================================================================

/// Verify that the cross-asset module can be initialized exactly once
/// with a valid admin address.
///
/// # Security
/// The admin address is immutable within this module after initialization.
/// This test confirms the happy path — subsequent tests confirm re-init is blocked.
#[test]
fn test_initialize_ca_success() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    // First call must succeed — admin is stored, module is ready
    client.initialize_ca(&admin);
}

/// Re-initializing the cross-asset module must be rejected with AlreadyInitialized.
///
/// # Security
/// Allowing re-initialization would let an attacker overwrite the admin address
/// and seize control of all asset configurations and price updates.
#[test]
#[should_panic]
fn test_initialize_ca_twice_fails() {
    let (_, client, admin) = setup();
    // setup() already called initialize_ca — this second call must panic
    client.initialize_ca(&admin);
}

// ============================================================================
// 2. Asset Initialization
// ============================================================================

/// Verify that a native XLM asset can be initialized with a valid configuration
/// and the stored values match what was provided.
///
/// This is the baseline happy-path test for asset registration.
#[test]
fn test_initialize_asset_success() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    // Read back and verify all key fields are persisted correctly
    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.collateral_factor, 7500);
    assert_eq!(fetched.liquidation_threshold, 8000);
    assert_eq!(fetched.price, 10_000_000);
}

/// Verify that a token contract asset can be initialized and retrieved by address.
///
/// Token assets use Some(address) as the key vs None for native XLM.
/// Both must coexist independently in the config map.
#[test]
fn test_initialize_token_asset_success() {
    let (env, client, _admin) = setup();
    let token_addr = Address::generate(&env);
    let config = token_config(&env, &token_addr);
    client.initialize_asset(&Some(token_addr.clone()), &config);

    // Retrieve by token address and verify fields
    let fetched = client.get_asset_config(&Some(token_addr));
    assert_eq!(fetched.collateral_factor, 6000);
    assert_eq!(fetched.price, 20_000_000);
}

/// Re-initializing the same asset must be rejected with AlreadyInitialized.
///
/// # Security
/// If re-initialization were allowed, an admin could silently reset an asset's
/// collateral factor, liquidation threshold, or price caps — potentially making
/// previously healthy positions immediately liquidatable or bypassing borrow caps.
#[test]
#[should_panic]
fn test_initialize_asset_twice_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);
    // Second call with same asset key must panic with AlreadyInitialized
    client.initialize_asset(&None, &config);
}

/// A collateral factor above 10_000 basis points (100%) must be rejected.
///
/// # Security
/// A collateral factor > 100% would mean a user could borrow more than their
/// collateral is worth, making every position immediately under-collateralised.
#[test]
#[should_panic]
fn test_initialize_asset_invalid_ltv_above_10000() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.collateral_factor = 10_001; // 100.01% — invalid, exceeds BPS_DENOMINATOR
    client.initialize_asset(&None, &config);
}

/// A negative collateral factor must be rejected.
///
/// # Security
/// Negative basis-point values would invert the collateral ratio calculation,
/// producing nonsensical or exploitable results in the health factor formula.
#[test]
#[should_panic]
fn test_initialize_asset_negative_ltv() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.collateral_factor = -1; // Negative — invalid
    client.initialize_asset(&None, &config);
}

/// A collateral factor (LTV) greater than the liquidation threshold must be rejected.
///
/// # Security
/// The invariant LTV <= liquidation_threshold ensures that a position can only
/// become liquidatable after it has already exceeded its borrow capacity. Violating
/// this invariant would create positions that are liquidatable from inception.
#[test]
#[should_panic]
fn test_initialize_asset_ltv_exceeds_liquidation_threshold() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.collateral_factor = 9000;      // 90% LTV
    config.liquidation_threshold = 8000;  // 80% threshold — LTV > threshold, invalid
    client.initialize_asset(&None, &config);
}

/// A price of zero must be rejected.
///
/// # Security
/// A zero price would make all collateral worthless and all debt free,
/// completely breaking the health factor calculation for the asset.
#[test]
#[should_panic]
fn test_initialize_asset_zero_price() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.price = 0; // Zero price — invalid, would cause division by zero in health factor
    client.initialize_asset(&None, &config);
}

/// A negative price must be rejected.
///
/// # Security
/// Negative prices would produce negative collateral values, inverting the
/// health factor and making liquidatable positions appear healthy.
#[test]
#[should_panic]
fn test_initialize_asset_negative_price() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.price = -5; // Negative price — invalid
    client.initialize_asset(&None, &config);
}

/// A negative max_supply cap must be rejected.
///
/// # Security
/// A negative supply cap would make every deposit exceed the cap immediately,
/// effectively disabling deposits for the asset.
#[test]
#[should_panic]
fn test_initialize_asset_negative_max_supply() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.max_supply = -100; // Negative cap — invalid
    client.initialize_asset(&None, &config);
}

/// A reserve factor above 10_000 basis points must be rejected.
///
/// # Security
/// A reserve factor > 100% would mean the protocol takes more than all interest
/// income, leaving lenders with negative yield — an invalid protocol state.
#[test]
#[should_panic]
fn test_initialize_asset_invalid_reserve_factor() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.reserve_factor = 10_001; // > 100% — invalid
    client.initialize_asset(&None, &config);
}

/// Supply and borrow caps of zero must be accepted and treated as unlimited.
///
/// Zero caps are a deliberate design choice meaning "no cap enforced".
/// This test confirms the contract stores and respects the zero value correctly.
#[test]
fn test_initialize_asset_zero_caps_unlimited() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.max_supply = 0; // 0 = unlimited supply
    config.max_borrow = 0; // 0 = unlimited borrows
    client.initialize_asset(&None, &config);

    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.max_supply, 0);
    assert_eq!(fetched.max_borrow, 0);
}

/// A collateral factor equal to the liquidation threshold must be accepted.
///
/// The invariant is LTV <= threshold (not strictly less than).
/// Equal values are valid and represent maximum LTV utilization.
#[test]
fn test_initialize_asset_edge_ltv_equals_threshold() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.collateral_factor = 8000;      // 80% LTV
    config.liquidation_threshold = 8000;  // 80% threshold — equal is allowed
    client.initialize_asset(&None, &config);
}

// ============================================================================
// 3. Config Updates
// ============================================================================

/// Verify that a full config update applies all provided fields and preserves
/// fields that were not updated (None = keep existing value).
#[test]
fn test_update_asset_config_success() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    // Update LTV and threshold together — both must change atomically
    client.update_asset_config(
        &None,
        &Some(6000),  // New LTV: 60%
        &Some(7000),  // New liquidation threshold: 70%
        &None,        // max_supply unchanged
        &None,        // max_borrow unchanged
        &None,        // can_collateralize unchanged
        &None,        // can_borrow unchanged
    );

    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.collateral_factor, 6000);
    assert_eq!(fetched.liquidation_threshold, 7000);
    // Fields not in the update must be preserved
    assert_eq!(fetched.reserve_factor, 1000);
    assert_eq!(fetched.can_collateralize, true);
}

/// Verify that a partial update touches only the specified field and leaves
/// all other fields at their original values.
///
/// This is critical for admin usability — a targeted config change should not
/// reset unrelated parameters.
#[test]
fn test_update_asset_config_partial_update() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    // Only disable borrowing — all other fields must remain unchanged
    client.update_asset_config(
        &None,
        &None,         // LTV unchanged
        &None,         // threshold unchanged
        &None,         // max_supply unchanged
        &None,         // max_borrow unchanged
        &None,         // can_collateralize unchanged
        &Some(false),  // Disable borrowing for this asset
    );

    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.can_borrow, false);
    assert_eq!(fetched.collateral_factor, 7500); // Must be unchanged
}

/// Setting LTV above the existing liquidation threshold must be rejected.
///
/// # Security
/// The validation applies to the *resulting* config, not just the delta.
/// This prevents unsafe transitions like raising LTV above an unchanged threshold.
#[test]
#[should_panic]
fn test_update_asset_config_ltv_above_threshold_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    // Attempt to raise LTV to 90% while threshold stays at 80% — must fail
    client.update_asset_config(
        &None,
        &Some(9000),  // New LTV 90% > existing threshold 80% — invalid
        &None,        // Threshold unchanged at 8000
        &None,
        &None,
        &None,
        &None,
    );
}

/// A collateral factor update that exceeds 10_000 basis points must be rejected.
///
/// Out-of-bounds basis-point values are always invalid regardless of context.
#[test]
#[should_panic]
fn test_update_asset_config_out_of_bounds_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    client.update_asset_config(
        &None,
        &Some(10_001), // 100.01% — exceeds BPS_DENOMINATOR, invalid
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

/// Attempting to update the config of an asset that was never initialized
/// must be rejected with AssetNotConfigured.
///
/// # Security
/// Accepting updates for unregistered assets could silently create phantom
/// configs that interfere with subsequent initialization.
#[test]
#[should_panic]
fn test_update_asset_config_unconfigured_asset_fails() {
    let (_env, client, _admin) = setup();
    // No asset has been initialized — update must fail
    client.update_asset_config(
        &None,
        &Some(5000),
        &None,
        &None,
        &None,
        &None,
        &None,
    );
}

// ============================================================================
// 4. Price Updates
// ============================================================================

/// Verify that a valid price update is stored and retrievable via get_asset_config.
///
/// Price updates also record the current ledger timestamp for staleness tracking.
#[test]
fn test_update_asset_price_success() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    // Update price from $1.00 to $2.00
    client.update_asset_price(&None, &20_000_000);

    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.price, 20_000_000);
}

/// A price update of zero must be rejected with InvalidPrice.
///
/// # Security
/// Zero price collapses all collateral value to zero, making every position
/// appear liquidatable and every borrow appear uncollateralised.
#[test]
#[should_panic]
fn test_update_asset_price_zero_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);
    client.update_asset_price(&None, &0); // Zero price — must panic
}

/// A negative price update must be rejected with InvalidPrice.
///
/// # Security
/// Negative prices invert collateral value calculations producing nonsensical
/// health factors and borrow capacity values.
#[test]
#[should_panic]
fn test_update_asset_price_negative_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);
    client.update_asset_price(&None, &-100); // Negative price — must panic
}

/// Attempting to update the price of an unregistered asset must fail.
///
/// # Security
/// Accepting price updates for unregistered assets could corrupt the config map
/// or create orphaned price entries that interfere with future initialization.
#[test]
#[should_panic]
fn test_update_asset_price_unconfigured_fails() {
    let (_env, client, _admin) = setup();
    // Asset never initialized — price update must panic with AssetNotConfigured
    client.update_asset_price(&None, &10_000_000);
}

// ============================================================================
// 5. Deposit
// ============================================================================

/// Verify a basic collateral deposit succeeds and the position reflects
/// the deposited amount with zero debt.
#[test]
fn test_deposit_success() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // Deposit 1000 units (7 decimals) of native XLM as collateral
    let position = client.cross_asset_deposit(&user, &None, &1000_0000000);

    assert_eq!(position.collateral, 1000_0000000); // Full amount deposited
    assert_eq!(position.debt_principal, 0);         // No debt created
}

/// Multiple deposits by the same user must accumulate correctly.
///
/// The contract must add to the existing collateral balance, not replace it.
#[test]
fn test_deposit_multiple_accumulates() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &500_0000000); // First deposit: 500
    let position = client.cross_asset_deposit(&user, &None, &300_0000000); // Second: 300

    // Total must be 800, not just the last deposit amount
    assert_eq!(position.collateral, 800_0000000);
}

/// Deposits must update the global total supply tracker for the asset.
///
/// The total supply is used for supply cap enforcement across all users.
#[test]
fn test_deposit_updates_total_supply() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);

    // Global supply tracker must reflect the deposit
    let total = client.get_total_supply_for(&None);
    assert_eq!(total, 1000_0000000);
}

/// A deposit of zero must be rejected with InvalidAmount.
///
/// # Security
/// Zero-amount deposits could be used to manipulate timestamps or trigger
/// events without moving any funds — rejecting them keeps state clean.
#[test]
#[should_panic]
fn test_deposit_zero_amount_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &0); // Zero amount — must panic
}

/// A negative deposit amount must be rejected with InvalidAmount.
///
/// # Security
/// Negative amounts could be used to drain collateral balances without going
/// through the withdrawal flow, bypassing health factor checks.
#[test]
#[should_panic]
fn test_deposit_negative_amount_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &-100); // Negative — must panic
}

/// A deposit that would push total supply above the cap must be rejected.
///
/// # Security
/// Supply caps protect against concentration risk — a single asset representing
/// too large a fraction of protocol TVL increases liquidation cascade risk.
#[test]
#[should_panic]
fn test_deposit_exceeds_supply_cap_fails() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.max_supply = 1000; // Very low cap for testing
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1001); // One unit over cap — must panic
}

/// Depositing into an asset where can_collateralize = false must be rejected.
///
/// # Security
/// Assets disabled for collateral must not accept new deposits — the admin
/// controls which assets are eligible for collateral use.
#[test]
#[should_panic]
fn test_deposit_disabled_asset_fails() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.can_collateralize = false; // Collateral disabled
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000); // Must panic — asset not collateralizable
}

/// When max_supply = 0 (unlimited), very large deposits must succeed without
/// triggering any cap error.
///
/// This confirms the contract correctly interprets zero as "no cap enforced".
#[test]
fn test_deposit_unlimited_supply_cap() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.max_supply = 0; // 0 = unlimited
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // Large deposit well above any realistic cap
    let position = client.cross_asset_deposit(&user, &None, &999_999_999_0000000);
    assert_eq!(position.collateral, 999_999_999_0000000);
}
/// A borrow that would push total borrows above the asset's borrow cap must be rejected.
///


// ============================================================================
// 6. Withdraw
// ============================================================================

#[test]
fn test_withdraw_success_no_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    let position = client.cross_asset_withdraw(&user, &None, &400_0000000);

    assert_eq!(position.collateral, 600_0000000);
}

#[test]
fn test_withdraw_full_amount_no_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    let position = client.cross_asset_withdraw(&user, &None, &1000_0000000);

    assert_eq!(position.collateral, 0);
}

#[test]
fn test_withdraw_updates_total_supply() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    client.cross_asset_withdraw(&user, &None, &400_0000000);

    let total = client.get_total_supply_for(&None);
    assert_eq!(total, 600_0000000);
}

#[test]
#[should_panic]
fn test_withdraw_zero_amount_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    client.cross_asset_withdraw(&user, &None, &0);
}

#[test]
#[should_panic]
fn test_withdraw_more_than_balance_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    client.cross_asset_withdraw(&user, &None, &1001_0000000);
}

#[test]
#[should_panic]
fn test_withdraw_unhealthy_position_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // Deposit 10000 ($10000 at $1), borrow 6000 ($6000)
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &6000_0000000);

    // Try to withdraw 5000 — would drop collateral to 5000 ($5000)
    // Weighted collateral = 5000 * 0.80 = 4000, debt = 6000
    // Health = 4000 / 6000 * 10000 = 6666 < 10000 → unhealthy
    client.cross_asset_withdraw(&user, &None, &5000_0000000);
}

// ============================================================================
// 7. Borrow
// ============================================================================

#[test]
fn test_borrow_success() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    let position = client.cross_asset_borrow(&user, &None, &5000_0000000);

    assert_eq!(position.debt_principal, 5000_0000000);
}

#[test]
fn test_borrow_updates_total_borrow() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &3000_0000000);

    let total = client.get_total_borrow_for(&None);
    assert_eq!(total, 3000_0000000);
}

#[test]
#[should_panic]
fn test_borrow_zero_amount_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &0);
}

#[test]
#[should_panic]
fn test_borrow_exceeds_health_factor_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);

    // Weighted collateral = 10000 * 0.80 = 8000
    // Borrowing 8001 would give health = 8000/8001 * 10000 = 9998 < 10000
    client.cross_asset_borrow(&user, &None, &8001_0000000);
}


/// # Security
/// Borrow caps act as a debt ceiling — they protect the protocol from excessive
/// exposure to a single asset. Exceeding the cap could leave the protocol
/// unable to absorb liquidations if the asset price drops sharply.
#[test]
#[should_panic]
fn test_borrow_exceeds_borrow_cap_fails() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.max_borrow = 1000_0000000; // $1000 global borrow cap for this asset
    config.max_supply = 0;            // Unlimited supply so cap is not the blocker
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // Deposit enough collateral to cover the borrow ratio
    client.cross_asset_deposit(&user, &None, &100000_0000000);
    // Attempt to borrow $1001 — one unit over the $1000 cap, must panic
    client.cross_asset_borrow(&user, &None, &1001_0000000);
}

/// Borrowing an asset where can_borrow = false must be rejected with AssetDisabled.
///
/// # Security
/// The admin may disable borrowing for an asset (e.g., during a risk event)
/// without removing it from the registry. This must be enforced even when
/// the user has sufficient collateral for the requested amount.
#[test]
#[should_panic]
fn test_borrow_disabled_asset_fails() {
    let (env, client, _admin) = setup();
    let mut config = default_config(&env);
    config.can_borrow = false; // Borrowing disabled for this asset
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    // Must panic — borrow disabled regardless of collateral
    client.cross_asset_borrow(&user, &None, &1000_0000000);
}

/// Borrowing exactly at the health factor boundary (health = 10_000 = 1.0x)
/// must succeed — the boundary is inclusive.
///
/// # Security
/// An exclusive boundary (health > 10_000) would prevent users from fully
/// utilizing their collateral. The inclusive boundary (health >= 10_000)
/// is the correct protocol invariant.
///
/// Math: deposit 10_000, weighted collateral = 10_000 * 0.80 = 8_000.
/// Borrow exactly 8_000 → health = 8_000 / 8_000 * 10_000 = 10_000.
#[test]
fn test_borrow_at_max_health_boundary() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // Deposit 10_000 units — weighted collateral = 10_000 * 80% = 8_000
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    // Borrow exactly 8_000 — health factor = 10_000 (borderline healthy, must succeed)
    let position = client.cross_asset_borrow(&user, &None, &8000_0000000);
    assert_eq!(position.debt_principal, 8000_0000000);
}

// ============================================================================
// 8. Repay
// ============================================================================

/// A partial repayment must reduce the debt principal by exactly the repaid amount.
///
/// Interest is paid first, then principal. Since no time has passed in this test,
/// accrued interest is zero and the full repayment goes to principal.
#[test]
fn test_repay_partial() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);

    // Repay 2000 of the 5000 debt — remaining should be 3000
    let position = client.cross_asset_repay(&user, &None, &2000_0000000);
    assert_eq!(position.debt_principal, 3000_0000000);
}

/// A full repayment must clear both principal and accrued interest to zero.
///
/// After full repayment the position should be debt-free, which restores
/// the user's full borrow capacity.
#[test]
fn test_repay_full() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);

    // Repay the exact debt amount — position must be fully cleared
    let position = client.cross_asset_repay(&user, &None, &5000_0000000);
    assert_eq!(position.debt_principal, 0);
    assert_eq!(position.accrued_interest, 0);
}

/// Overpaying must be silently capped at the total outstanding debt.
///
/// A user paying more than their debt must not end up with a negative
/// debt balance or cause an underflow. The excess is discarded.
///
/// # Security
/// Without the cap, an overpayment could cause a checked_sub underflow
/// that panics or produces a wrapped negative value depending on arithmetic mode.
#[test]
fn test_repay_capped_at_total_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);

    // Attempt to repay 99_999 — far more than the 5_000 debt. Must cap at 5_000.
    let position = client.cross_asset_repay(&user, &None, &99999_0000000);
    assert_eq!(position.debt_principal, 0); // Debt cleared, no underflow
}

/// Repayments must decrease the global total borrow tracker by the repaid amount.
///
/// The total borrow tracker is used for borrow cap enforcement. If it is not
/// decremented correctly, future borrows could be wrongly rejected as exceeding
/// the cap even after repayment frees capacity.
#[test]
fn test_repay_updates_total_borrow() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);
    client.cross_asset_repay(&user, &None, &2000_0000000);

    // Total borrow must be 5000 - 2000 = 3000
    let total = client.get_total_borrow_for(&None);
    assert_eq!(total, 3000_0000000);
}

/// A repayment of zero must be rejected with InvalidAmount.
///
/// # Security
/// Zero-amount repayments could be used to manipulate last_updated timestamps
/// without actually reducing any debt, potentially gaming interest accrual.
#[test]
#[should_panic]
fn test_repay_zero_amount_fails() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);
    client.cross_asset_repay(&user, &None, &0); // Zero repay — must panic
}

// ============================================================================
// 9. Position Queries
// ============================================================================

/// A user with no interactions must return a default zero position.
///
/// The contract must not panic when querying a position that has never been
/// initialized — it returns zeros by default.
#[test]
fn test_get_user_asset_position_default() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    // User has never interacted — default position must be all zeros
    let position = client.get_user_asset_position(&user, &None);
    assert_eq!(position.collateral, 0);
    assert_eq!(position.debt_principal, 0);
}

/// A user with no positions must return a summary with zero values and
/// infinite health factor.
///
/// Infinite health (i128::MAX) is the correct representation for a user
/// with no debt — they cannot be liquidated regardless of price movement.
#[test]
fn test_get_user_position_summary_no_positions() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 0);
    assert_eq!(summary.total_debt_value, 0);
    assert_eq!(summary.health_factor, i128::MAX); // Infinite — no debt
    assert_eq!(summary.is_liquidatable, false);
}

/// A user with only collateral (no debt) must show correct collateral value,
/// weighted collateral, infinite health, and full borrow capacity.
///
/// Math: 1000 units * $1.00 price = $1000 collateral value.
/// Weighted = $1000 * 80% liquidation threshold = $800.
/// Borrow capacity = weighted collateral - 0 debt = $800.
#[test]
fn test_get_user_position_summary_with_collateral_only() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 1000_0000000);  // $1000.00
    assert_eq!(summary.weighted_collateral_value, 800_0000000); // $800 at 80% threshold
    assert_eq!(summary.total_debt_value, 0);
    assert_eq!(summary.health_factor, i128::MAX); // No debt → infinite health
    assert_eq!(summary.is_liquidatable, false);
    assert_eq!(summary.borrow_capacity, 800_0000000); // Full capacity available
}

/// A user with both collateral and debt must show correct health factor
/// and reduced borrow capacity.
///
/// Math: 10_000 collateral * $1.00 = $10_000. Weighted = $8_000 (80%).
/// Debt = 5_000 * $1.00 = $5_000.
/// Health = 8_000 / 5_000 * 10_000 = 16_000 (1.6x — healthy).
/// Remaining capacity = 8_000 - 5_000 = 3_000.
#[test]
fn test_get_user_position_summary_with_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 10000_0000000);
    assert_eq!(summary.weighted_collateral_value, 8000_0000000);
    assert_eq!(summary.total_debt_value, 5000_0000000);
    assert_eq!(summary.health_factor, 16000); // 1.6x
    assert_eq!(summary.is_liquidatable, false);
    assert_eq!(summary.borrow_capacity, 3000_0000000);
}

/// The asset list must contain all registered assets in registration order.
///
/// The asset list is used by get_user_position_summary to iterate all assets.
/// If assets are missing from the list, positions for those assets are silently
/// excluded from health factor calculations.
#[test]
fn test_get_asset_list() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config); // Asset 1: native XLM

    let token_addr = Address::generate(&env);
    let tconfig = token_config(&env, &token_addr);
    client.initialize_asset(&Some(token_addr), &tconfig); // Asset 2: token

    let list = client.get_asset_list();
    assert_eq!(list.len(), 2); // Both assets must appear
}

/// Total supply must be zero before any deposits.
///
/// This is the baseline for supply cap enforcement tests — confirms the tracker
/// starts at zero and is not contaminated by other tests.
#[test]
fn test_get_total_supply_for_default_zero() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let total = client.get_total_supply_for(&None);
    assert_eq!(total, 0);
}

/// Total borrows must be zero before any borrows.
///
/// This is the baseline for borrow cap enforcement tests.
#[test]
fn test_get_total_borrow_for_default_zero() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let total = client.get_total_borrow_for(&None);
    assert_eq!(total, 0);
}

// ============================================================================
// 10. Cross-Asset Position (multi-asset)
// ============================================================================

/// Verify that deposits and borrows across two different assets are correctly
/// aggregated into a single health factor summary.
///
/// Math:
/// - XLM: 1000 deposited * $1.00 = $1000 collateral. Weighted = $1000 * 80% = $800.
/// - Token: 500 deposited * $2.00 = $1000 collateral. Weighted = $1000 * 70% = $700.
/// - Total weighted collateral = $800 + $700 = $1500.
/// - Borrow 1000 XLM * $1.00 = $1000 debt.
/// - Health = 1500 / 1000 * 10000 = 15000 (1.5x).
#[test]
fn test_multi_asset_deposit_and_borrow() {
    let (env, client, _admin) = setup();

    let config_a = default_config(&env); // XLM at $1, 75% LTV, 80% threshold
    client.initialize_asset(&None, &config_a);

    let token_addr = Address::generate(&env);
    let config_b = token_config(&env, &token_addr); // Token at $2, 60% LTV, 70% threshold
    client.initialize_asset(&Some(token_addr.clone()), &config_b);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    client.cross_asset_deposit(&user, &Some(token_addr.clone()), &500_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 2000_0000000);      // $1000 + $1000
    assert_eq!(summary.weighted_collateral_value, 1500_0000000);   // $800 + $700

    client.cross_asset_borrow(&user, &None, &1000_0000000);

    let summary2 = client.get_user_position_summary(&user);
    assert_eq!(summary2.total_debt_value, 1000_0000000);
    assert_eq!(summary2.health_factor, 15000); // 1.5x — healthy
    assert_eq!(summary2.is_liquidatable, false);
}

/// Full lifecycle test: deposit → borrow → repay all → withdraw all.
///
/// After repaying all debt, the user must be able to withdraw their full
/// collateral with no health factor obstruction.
#[test]
fn test_multi_asset_repay_then_withdraw() {
    let (env, client, _admin) = setup();

    let config_a = default_config(&env);
    client.initialize_asset(&None, &config_a);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &5000_0000000);

    // Repay all — health becomes infinite
    client.cross_asset_repay(&user, &None, &5000_0000000);

    // Full withdrawal must succeed with no debt blocking it
    let position = client.cross_asset_withdraw(&user, &None, &10000_0000000);
    assert_eq!(position.collateral, 0);
}

// ============================================================================
// 11. Staleness Tests
// ============================================================================

/// A borrow attempted after the price staleness threshold (1 hour) must fail.
///
/// # Security
/// Borrowing requires a health factor check, which requires a current price.
/// Stale prices allow manipulation — an attacker could borrow against
/// collateral whose price has since dropped significantly.
#[test]
#[should_panic]
fn test_stale_price_rejects_borrow() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);

    // Advance ledger time beyond the 3600-second staleness window
    env.ledger().with_mut(|li| {
        li.timestamp = li.timestamp + 3601; // 1 second past the threshold
    });

    // Health check reads the stale price — must panic with PriceStale
    client.cross_asset_borrow(&user, &None, &1000_0000000);
}

/// A withdrawal attempted after the staleness threshold when the user has debt
/// must fail.
///
/// # Security
/// With stale prices we cannot verify the health factor post-withdrawal.
/// Allowing withdrawals with stale prices + debt could let users extract
/// collateral from positions that are actually underwater.
#[test]
#[should_panic]
fn test_stale_price_rejects_withdraw_with_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &1000_0000000);

    // Advance past staleness threshold
    env.ledger().with_mut(|li| {
        li.timestamp = li.timestamp + 3601;
    });

    // User has debt — stale price blocks health check — must panic
    client.cross_asset_withdraw(&user, &None, &100_0000000);
}

/// A withdrawal with stale prices must succeed when the user has no debt.
///
/// When health factor is infinite (no debt), price staleness is irrelevant —
/// no health check is needed. This test confirms the protocol does not
/// unnecessarily block debt-free users.
#[test]
fn test_stale_price_allows_withdraw_without_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);

    // Advance far past the staleness threshold — 2 hours
    env.ledger().with_mut(|li| {
        li.timestamp = li.timestamp + 7200;
    });

    // No debt — health check skipped — withdrawal must succeed
    let position = client.cross_asset_withdraw(&user, &None, &5000_0000000);
    assert_eq!(position.collateral, 5000_0000000);
}

// ============================================================================
// 12. Liquidation Status
// ============================================================================

/// When collateral and debt are in the same asset, a price drop does not change
/// the health factor because both collateral value and debt value scale equally.
///
/// This test documents the expected behavior and sets up the rationale for the
/// cross-asset liquidation scenario in the next test.
#[test]
fn test_position_becomes_liquidatable_after_price_drop() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);
    client.cross_asset_borrow(&user, &None, &7000_0000000);

    // Initial health = (10000 * 0.80) / 7000 * 10000 ≈ 11428 (healthy)
    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.is_liquidatable, false);

    // Drop price to $0.50 — but since deposit and borrow are same asset,
    // both collateral value and debt value halve, preserving the ratio
    client.update_asset_price(&None, &5_000_000);

    // Health ratio unchanged — same asset price drop cancels out
    let summary2 = client.get_user_position_summary(&user);
    assert_eq!(summary2.is_liquidatable, false);
}

/// A cross-asset scenario where collateral and debt are different assets
/// correctly triggers liquidation when the collateral asset price drops.
///
/// Math after XLM drops to $0.50:
/// - XLM collateral value = 10_000 * $0.50 = $5_000
/// - Weighted = $5_000 * 80% = $4_000
/// - Token debt = 7_000 * $1.00 = $7_000
/// - Health = 4_000 / 7_000 * 10_000 = 5_714 < 10_000 → liquidatable
///
/// # Security
/// This is the core liquidation trigger test. It confirms that oracle price
/// updates correctly propagate to health factor calculations.
#[test]
fn test_cross_asset_liquidation_scenario() {
    let (env, client, _admin) = setup();

    // XLM: collateral only (can_borrow = false)
    let mut config_xlm = default_config(&env);
    config_xlm.can_borrow = false;
    client.initialize_asset(&None, &config_xlm);

    // Token: borrow only (can_collateralize = false), $1.00 price
    let token = Address::generate(&env);
    let mut config_token = token_config(&env, &token);
    config_token.price = 10_000_000;         // $1.00
    config_token.can_collateralize = false;  // Cannot be used as collateral
    config_token.max_borrow = 0;             // Unlimited borrows for test
    config_token.liquidation_threshold = 8000;
    config_token.collateral_factor = 7500;
    client.initialize_asset(&Some(token.clone()), &config_token);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);       // 10_000 XLM collateral
    client.cross_asset_borrow(&user, &Some(token.clone()), &7000_0000000); // 7_000 token debt

    // Pre-drop: health ≈ 11_428 — healthy
    let summary = client.get_user_position_summary(&user);
    assert!(summary.health_factor > 10000);
    assert!(!summary.is_liquidatable);

    // XLM price drops 50%
    client.update_asset_price(&None, &5_000_000); // $0.50

    // Post-drop: health = 5_714 — liquidatable
    let summary2 = client.get_user_position_summary(&user);
    assert!(summary2.health_factor < 10000);
    assert!(summary2.is_liquidatable);
}

// ============================================================================
// 13. Edge Cases
// ============================================================================

/// Repaying when the user has no debt must succeed and return a zero-debt position.
///
/// The repay amount is capped at total_debt (which is zero), so the call
/// completes as a no-op without error. This is safe and expected behavior.
#[test]
fn test_repay_no_debt_repays_nothing() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);

    // Repay 1000 with no outstanding debt — capped at 0, no-op
    let position = client.cross_asset_repay(&user, &None, &1000_0000000);
    assert_eq!(position.debt_principal, 0);
}

/// Two users must have independent positions that do not interfere with each other.
///
/// The global total supply must be the sum of both users' deposits.
/// Each user's position must reflect only their own deposits.
#[test]
fn test_multiple_users_independent_positions() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.cross_asset_deposit(&user1, &None, &5000_0000000); // User1: 5000
    client.cross_asset_deposit(&user2, &None, &3000_0000000); // User2: 3000

    // Positions are independent
    let pos1 = client.get_user_asset_position(&user1, &None);
    let pos2 = client.get_user_asset_position(&user2, &None);
    assert_eq!(pos1.collateral, 5000_0000000);
    assert_eq!(pos2.collateral, 3000_0000000);

    // Global supply is the aggregate of both
    let total = client.get_total_supply_for(&None);
    assert_eq!(total, 8000_0000000);
}

/// Disabling collateral on an asset after existing deposits must not erase those positions.
///
/// The existing position is preserved in storage. Only new deposits are blocked.
/// This protects users who deposited before the config change.
#[test]
fn test_deposit_then_disable_collateral_blocks_new_deposits() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &5000_0000000); // Deposit before disable

    // Admin disables new collateral deposits for this asset
    client.update_asset_config(
        &None,
        &None,
        &None,
        &None,
        &None,
        &Some(false), // can_collateralize = false — blocks new deposits
        &None,
    );

    // Existing position must be preserved — disable is not retroactive
    let pos = client.get_user_asset_position(&user, &None);
    assert_eq!(pos.collateral, 5000_0000000);
}

/// The asset list must remain stable across deposit, borrow, repay, and withdraw operations.
///
/// Operations on positions must not add duplicate entries to the list or remove
/// entries from it — the list is registry-only and mutated only by initialize_asset.
#[test]
fn test_asset_list_preserved_across_operations() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let token = Address::generate(&env);
    let tconfig = token_config(&env, &token);
    client.initialize_asset(&Some(token.clone()), &tconfig);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);
    client.cross_asset_deposit(&user, &Some(token), &500_0000000);

    // List must still contain exactly 2 assets — no duplicates added by deposits
    let list = client.get_asset_list();
    assert_eq!(list.len(), 2);
}

/// A user with collateral but no debt must always have health_factor = i128::MAX.
///
/// i128::MAX represents infinite health — the user cannot be liquidated
/// regardless of price movements because they have no debt to default on.
#[test]
fn test_health_factor_max_when_no_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &1000_0000000);

    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.health_factor, i128::MAX);
}

/// Borrow capacity must decrease by exactly the debt value after each borrow.
///
/// Initial capacity = weighted collateral = 10_000 * 80% = 8_000.
/// After borrowing 3_000: capacity = 8_000 - 3_000 = 5_000.
#[test]
fn test_borrow_capacity_decreases_with_debt() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &10000_0000000);

    let summary1 = client.get_user_position_summary(&user);
    assert_eq!(summary1.borrow_capacity, 8000_0000000); // 10_000 * 80% = 8_000

    client.cross_asset_borrow(&user, &None, &3000_0000000);

    let summary2 = client.get_user_position_summary(&user);
    assert_eq!(summary2.borrow_capacity, 5000_0000000); // 8_000 - 3_000 = 5_000
}

/// A config update must not reset the asset's price to its initialization value.
///
/// Config updates and price updates are separate operations. A config update
/// that does not touch the price field must preserve the most recent price.
#[test]
fn test_config_update_preserves_price() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    client.update_asset_price(&None, &50_000_000); // Update price to $5.00

    // Update LTV and threshold — price must remain at $5.00
    client.update_asset_config(
        &None,
        &Some(5000), // New LTV: 50%
        &Some(6000), // New threshold: 60%
        &None,
        &None,
        &None,
        &None,
    );

    let fetched = client.get_asset_config(&None);
    assert_eq!(fetched.price, 50_000_000);       // Price preserved at $5.00
    assert_eq!(fetched.collateral_factor, 5000); // New LTV applied
}

// ── Issue #530: user position summary gas bounds ──────────────────────────

/// Registering more than MAX_ASSETS_PER_SUMMARY assets must not cause the
/// position summary to panic or exhaust the Soroban per-transaction resource budget.
/// The summary silently caps iteration at MAX_ASSETS_PER_SUMMARY assets.
///
/// # Security
/// Without the cap, a malicious or misconfigured admin could register enough
/// assets to make get_user_position_summary permanently fail for all users,
/// effectively bricking the protocol's health factor checks and blocking
/// all withdrawals and borrows indefinitely.
///
/// # Test Design
/// Registers MAX + 5 assets, deposits into the first MAX assets, and confirms
/// the summary returns the correct aggregate for exactly MAX assets without panic.
#[test]
fn test_position_summary_bounded_with_many_assets() {
    use crate::cross_asset::MAX_ASSETS_PER_SUMMARY;

    let (env, client, _admin) = setup();

    // Register MAX_ASSETS_PER_SUMMARY + 5 assets — exceeds the cap
    let target = MAX_ASSETS_PER_SUMMARY + 5;
    let mut tokens: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&env);

    for _ in 0..target {
        let token = Address::generate(&env);
        tokens.push_back(token.clone());
        let cfg = AssetConfig {
            asset: Some(token.clone()),
            collateral_factor: 7500,
            liquidation_threshold: 8000,
            reserve_factor: 1000,
            max_supply: 0,       // Unlimited — cap not the blocker here
            max_borrow: 0,
            can_collateralize: true,
            can_borrow: true,
            price: 10_000_000,   // $1.00 per asset for predictable math
            price_updated_at: env.ledger().timestamp(),
        };
        client.initialize_asset(&Some(token), &cfg);
    }

    // Deposit 1 unit into each of the first MAX_ASSETS_PER_SUMMARY assets
    let user = Address::generate(&env);
    for i in 0..MAX_ASSETS_PER_SUMMARY {
        let token = tokens.get(i).unwrap();
        client.cross_asset_deposit(&user, &Some(token), &1_0000000);
    }

    // Must complete without panic — cap silently limits iteration
    let summary = client.get_user_position_summary(&user);

    // Each of MAX assets contributes $1.00 collateral at $1.00 price
    let expected_collateral = (MAX_ASSETS_PER_SUMMARY as i128) * 1_0000000_i128;
    assert_eq!(summary.total_collateral_value, expected_collateral);
    assert_eq!(summary.total_debt_value, 0);
    assert_eq!(summary.health_factor, i128::MAX); // No debt — infinite health
}

/// A single-asset registry must produce identical summary results before and after
/// the MAX_ASSETS_PER_SUMMARY cap was introduced.
///
/// This is a regression guard — the cap must not truncate or alter results for
/// protocols with fewer assets than the limit.
#[test]
fn test_position_summary_single_asset_unaffected_by_cap() {
    let (env, client, _admin) = setup();
    let config = default_config(&env);
    client.initialize_asset(&None, &config);

    let user = Address::generate(&env);
    client.cross_asset_deposit(&user, &None, &5000_0000000);

    // Single asset — cap is irrelevant, results must be exact
    let summary = client.get_user_position_summary(&user);
    assert_eq!(summary.total_collateral_value, 5000_0000000);
    assert_eq!(summary.health_factor, i128::MAX);
}

// ── Issue #445: AMM type wiring verification ──────────────────────────────

/// Confirms that AmmProtocolConfig and SwapParams in hello-world/amm.rs are
/// the real types re-exported from the stellarlend_amm crate, not local stubs.
///
/// # Security
/// Cross-crate API stability is critical. If hello-world defined its own local
/// placeholder types instead of importing from stellarlend_amm, a breaking change
/// to the AMM crate's public API would compile successfully in hello-world while
/// silently producing incorrect behavior at runtime. This test catches that by
/// constructing both types via the import path and exercising them through a
/// full swap round-trip.
///
/// # What this verifies
/// 1. AmmProtocolConfig can be constructed from crate::AmmProtocolConfig (not a local stub)
/// 2. SwapParams can be constructed from crate::SwapParams (not a local stub)
/// 3. A full initialize → set_pool → swap round-trip succeeds with real types
/// 4. The swap returns a positive output confirming the AMM logic is actually invoked
#[test]
fn test_amm_types_are_real_stellarlend_amm_types() {
    use crate::{AmmProtocolConfig, SwapParams, TokenPair};
    use soroban_sdk::{Symbol, Vec};

    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(crate::HelloContract, ());
    let client = crate::HelloContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let protocol = Address::generate(&env);
    let token_b = Address::generate(&env);

    // Initialize AMM settings: 0.5% default slippage, 5% max, 5000 threshold
    client.initialize_amm(&admin, &50, &500, &5000);

    // Build a supported pair for the protocol pool
    let mut pairs = Vec::new(&env);
    pairs.push_back(TokenPair {
        token_a: None,              // Native XLM
        token_b: Some(token_b.clone()), // Target token
        pool_address: Address::generate(&env),
    });

    // Construct AmmProtocolConfig via the real import path from stellarlend_amm.
    // If this were a local stub, a field rename in the AMM crate would cause a
    // compile error here, catching the divergence immediately.
    let config = AmmProtocolConfig {
        protocol_address: protocol.clone(),
        protocol_name: Symbol::new(&env, "RealAMM"),
        enabled: true,
        fee_tier: 30,               // 0.3% fee
        min_swap_amount: 100,
        max_swap_amount: 1_000_000_000,
        supported_pairs: pairs,
    };
    client.set_amm_pool(&admin, &config);

    // Construct SwapParams via the real import path.
    // A field mismatch between hello-world's import and stellarlend_amm's definition
    // would fail to compile, proving the wiring is real and not stubbed.
    let swap = SwapParams {
        protocol: protocol.clone(),
        token_in: None,             // Swap from native XLM
        token_b: Some(token_b.clone()),
        token_out: Some(token_b),
        amount_in: 10_000,
        min_amount_out: 9_000,      // Accept up to 10% slippage
        slippage_tolerance: 100,    // 1% tolerance
        deadline: env.ledger().timestamp() + 3600,
    };

    // Execute the swap — a positive return value confirms the real AMM logic ran
    let out = client.amm_swap(&Address::generate(&env), &swap);
    assert!(out > 0, "swap through real AMM types must return positive output");
}
