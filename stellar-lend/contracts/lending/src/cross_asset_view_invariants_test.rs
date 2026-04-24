//! # Cross-Asset Position Summary Invariant Tests
//!
//! These tests verify that `get_cross_position_summary` and related view methods
//! remain consistent with the underlying per-asset balances and risk configuration
//! at all times.
//!
//! ## Invariants Asserted
//!
//! | # | Invariant |
//! |---|-----------|
//! | I-1 | An empty position returns zero totals and `HF_NO_DEBT`. |
//! | I-2 | `total_collateral_usd` equals the arithmetic sum of each asset's `amount × price ÷ divisor`. |
//! | I-3 | `total_debt_usd` equals the arithmetic sum of each debt asset's `amount × price ÷ divisor`. |
//! | I-4 | `health_factor = weighted_collateral × BPS_SCALE ÷ total_debt_usd` (integer floor). |
//! | I-5 | `health_factor = HF_NO_DEBT` (1 000 000) when `total_debt_usd == 0`. |
//! | I-6 | After a valid borrow `health_factor ≥ BPS_SCALE` (1.0). |
//! | I-7 | After a full repay `health_factor` returns to `HF_NO_DEBT`. |
//! | I-8 | Repeated reads of `get_cross_position_summary` return identical results (idempotent). |
//! | I-9 | Different users have fully isolated position summaries. |
//! | I-10 | Assets with higher LTV contribute more to borrow capacity (monotone in LTV). |
//! | I-11 | Depositing more collateral (all else equal) increases `health_factor` (monotone). |
//! | I-12 | Borrowing more (all else equal) decreases `health_factor` (monotone). |
//! | I-13 | Sums are associative: depositing assets in different orders yields the same `total_collateral_usd`. |
//! | I-14 | LTV rounding truncates toward zero (conservative — floor division). |
//! | I-15 | Multiple collateral assets sum independently, as do multiple debt assets. |
//!
//! ## Security Notes
//!
//! - **Views are read-only.** `get_cross_position_summary` never writes to storage; an
//!   attacker cannot cause state changes by calling it repeatedly or concurrently.
//! - **No float arithmetic.** All computations use integer arithmetic with explicit
//!   `checked_*` calls. There is no rounding ambiguity that could be exploited.
//! - **Oracle trust boundary.** The mock oracle used in these tests always returns
//!   $1.00 (= 10 000 000 with 7 decimals). In production a price-manipulation attack
//!   is possible if the oracle is compromised; TWAP and deviation guards mitigate this.
//! - **Admin-controlled parameters.** LTV and debt ceilings are set by the admin.
//!   Tests mock all auths to verify behaviour; no test bypasses authorisation checks.
//! - **No view-based exploitation.** Because the summary is derived entirely from
//!   on-chain storage at the moment of the call, there is no way to front-run or
//!   replay a view to gain an informational advantage that the protocol does not
//!   already expose via individual balance reads.

use crate::cross_asset::{AssetParams, CrossAssetError};
use crate::{LendingContract, LendingContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

// ─────────────────────────────────────────────────────────────────────────────
// Protocol constants (mirrors cross_asset.rs / constants.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// 100 % in basis points — also the health-factor scale where 1.0 = 10 000.
const BPS_SCALE: i128 = 10_000;
/// Mock price returned by `get_price` for every asset: $1.00 with 7 decimals.
const MOCK_PRICE: i128 = 10_000_000;
/// Divisor used in `calculate_position_summary` to convert raw × price → USD value.
const PRICE_DIVISOR: i128 = 10_000_000;
/// Sentinel health factor when no debt exists.
const HF_NO_DEBT: i128 = 1_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Deploy the lending contract and initialise both the main protocol and the
/// cross-asset admin.
fn setup(env: &Env) -> (LendingContractClient<'_>, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1_000);
    client.initialize_admin(&admin);
    (client, admin)
}

/// Return an active `AssetParams` with the given `ltv`.
/// `liquidation_threshold` is set to `ltv + 500` (capped at BPS_SCALE) so the
/// constraint `ltv ≤ liquidation_threshold` is always satisfied.
fn make_params(env: &Env, ltv: i128) -> AssetParams {
    AssetParams {
        ltv,
        liquidation_threshold: (ltv + 500).min(BPS_SCALE),
        price_feed: Address::generate(env), // ignored by mock get_price
        debt_ceiling: 1_000_000_000_000,
        is_active: true,
    }
}

/// Register a new asset and return its address.
fn register_asset(env: &Env, client: &LendingContractClient<'_>, ltv: i128) -> Address {
    let asset = Address::generate(env);
    client.set_asset_params(&asset, &make_params(env, ltv));
    asset
}

/// Expected USD value of `amount` at the mock price (integer floor).
fn usd_value(amount: i128) -> i128 {
    amount * MOCK_PRICE / PRICE_DIVISOR
}

/// Expected weighted collateral contribution of `amount` with `ltv` (integer floor).
fn weighted(amount: i128, ltv: i128) -> i128 {
    usd_value(amount) * ltv / BPS_SCALE
}

/// Expected health factor given pre-calculated totals.
fn expected_hf(total_weighted: i128, total_debt: i128) -> i128 {
    if total_debt == 0 {
        HF_NO_DEBT
    } else {
        total_weighted * BPS_SCALE / total_debt
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I-1 — Empty position
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_empty_position_zero_totals_max_health() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    let s = client.get_cross_position_summary(&user);

    assert_eq!(s.total_collateral_usd, 0, "I-1: collateral must be 0");
    assert_eq!(s.total_debt_usd, 0, "I-1: debt must be 0");
    assert_eq!(s.health_factor, HF_NO_DEBT, "I-1: HF must be HF_NO_DEBT");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-2 — Collateral total matches sum of per-asset values
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_single_collateral_value_matches_amount_at_unit_price() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    let amount = 12_345;

    client.deposit_collateral_asset(&user, &asset, &amount);
    let s = client.get_cross_position_summary(&user);

    assert_eq!(s.total_collateral_usd, usd_value(amount), "I-2: single asset value");
}

#[test]
fn invariant_multi_collateral_total_is_sum_of_individual_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let a1 = register_asset(&env, &client, 7_500);
    let a2 = register_asset(&env, &client, 6_000);
    let a3 = register_asset(&env, &client, 5_000);
    let user = Address::generate(&env);

    let amt1 = 10_000i128;
    let amt2 = 5_000i128;
    let amt3 = 3_333i128;

    client.deposit_collateral_asset(&user, &a1, &amt1);
    client.deposit_collateral_asset(&user, &a2, &amt2);
    client.deposit_collateral_asset(&user, &a3, &amt3);

    let s = client.get_cross_position_summary(&user);
    let expected_collateral = usd_value(amt1) + usd_value(amt2) + usd_value(amt3);

    assert_eq!(s.total_collateral_usd, expected_collateral, "I-2: multi-asset sum");
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-3 — Debt total matches sum of per-asset debt values
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_single_debt_value_matches_borrow_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 9_000);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);

    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, usd_value(5_000), "I-3: single debt value");
}

#[test]
fn invariant_multi_debt_total_is_sum_of_individual_debt_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let a1 = register_asset(&env, &client, 9_000);
    let a2 = register_asset(&env, &client, 9_000);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &a1, &50_000);
    client.deposit_collateral_asset(&user, &a2, &50_000);

    let d1 = 10_000i128;
    let d2 = 20_000i128;
    client.borrow_asset(&user, &a1, &d1);
    client.borrow_asset(&user, &a2, &d2);

    let s = client.get_cross_position_summary(&user);
    let expected_debt = usd_value(d1) + usd_value(d2);

    assert_eq!(s.total_debt_usd, expected_debt, "I-3: multi-asset debt sum");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-4 — Health factor formula
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_health_factor_formula_single_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let ltv = 7_500i128;
    let asset = register_asset(&env, &client, ltv);
    let user = Address::generate(&env);
    let collateral = 10_000i128;
    let debt = 5_000i128;

    client.deposit_collateral_asset(&user, &asset, &collateral);
    client.borrow_asset(&user, &asset, &debt);

    let s = client.get_cross_position_summary(&user);

    let w = weighted(collateral, ltv);
    let d = usd_value(debt);
    let exp_hf = expected_hf(w, d);

    assert_eq!(s.health_factor, exp_hf, "I-4: HF formula (single asset)");
    // Sanity: with these numbers HF = 7500 * 10000 / 5000 = 15000
    assert_eq!(s.health_factor, 15_000);
}

#[test]
fn invariant_health_factor_formula_multi_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // Collateral A: 8 000 units, LTV 75 % → weighted = 6 000
    // Collateral B: 5 000 units, LTV 60 % → weighted = 3 000
    // Debt A:  4 000 units → debt_usd = 4 000
    // Debt B:  2 000 units → debt_usd = 2 000
    // total_weighted = 9 000; total_debt = 6 000
    // HF = 9000 * 10000 / 6000 = 15000

    let c_a = register_asset(&env, &client, 7_500);
    let c_b = register_asset(&env, &client, 6_000);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &c_a, &8_000);
    client.deposit_collateral_asset(&user, &c_b, &5_000);
    client.borrow_asset(&user, &c_a, &4_000);
    client.borrow_asset(&user, &c_b, &2_000);

    let s = client.get_cross_position_summary(&user);

    let total_weighted = weighted(8_000, 7_500) + weighted(5_000, 6_000);
    let total_debt = usd_value(4_000) + usd_value(2_000);
    let exp_hf = expected_hf(total_weighted, total_debt);

    assert_eq!(s.total_collateral_usd, usd_value(8_000) + usd_value(5_000));
    assert_eq!(s.total_debt_usd, total_debt);
    assert_eq!(s.health_factor, exp_hf, "I-4: HF formula (multi-asset)");
    assert_eq!(s.health_factor, 15_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-5 — HF_NO_DEBT when no debt
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_no_debt_yields_sentinel_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &100_000);

    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.health_factor, HF_NO_DEBT, "I-5: no debt → HF_NO_DEBT");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-6 — HF ≥ BPS_SCALE after a valid borrow
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_health_factor_always_ge_healthy_after_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &20_000);

    // Borrow at different fractions of the available capacity, then repay.
    for borrow_amount in [1i128, 1_000, 5_000, 7_500, 14_999] {
        client.borrow_asset(&user, &asset, &borrow_amount);

        let s = client.get_cross_position_summary(&user);
        assert!(
            s.health_factor >= BPS_SCALE,
            "I-6: HF must be ≥ BPS_SCALE after borrow of {borrow_amount} (got {})",
            s.health_factor,
        );

        // Repay to reset for the next iteration.
        client.repay_asset(&user, &asset, &borrow_amount);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I-7 — Full repay restores HF_NO_DEBT
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_full_repay_restores_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &5_000);

    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0, "I-7: debt cleared");
    assert_eq!(s.health_factor, HF_NO_DEBT, "I-7: HF restored to HF_NO_DEBT");
}

#[test]
fn invariant_overpay_also_restores_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &4_000);

    // Repay 2× the debt — should be capped at actual debt balance.
    client.repay_asset(&user, &asset, &8_000);

    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0, "I-7: overpay clears debt");
    assert_eq!(s.health_factor, HF_NO_DEBT, "I-7: overpay restores HF_NO_DEBT");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-8 — Idempotency (repeated reads)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_summary_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);

    let s1 = client.get_cross_position_summary(&user);
    let s2 = client.get_cross_position_summary(&user);
    let s3 = client.get_cross_position_summary(&user);

    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd, "I-8: collateral idempotent");
    assert_eq!(s1.total_debt_usd, s2.total_debt_usd, "I-8: debt idempotent");
    assert_eq!(s1.health_factor, s2.health_factor, "I-8: HF idempotent");
    assert_eq!(s2.health_factor, s3.health_factor, "I-8: HF stable across 3 reads");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-9 — User isolation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_users_have_isolated_positions() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);

    client.deposit_collateral_asset(&user_a, &asset, &20_000);
    client.borrow_asset(&user_a, &asset, &10_000);

    // user_b has done nothing.
    let sb = client.get_cross_position_summary(&user_b);
    assert_eq!(sb.total_collateral_usd, 0, "I-9: user B collateral unaffected");
    assert_eq!(sb.total_debt_usd, 0, "I-9: user B debt unaffected");
    assert_eq!(sb.health_factor, HF_NO_DEBT, "I-9: user B HF unaffected");

    let sa = client.get_cross_position_summary(&user_a);
    assert_eq!(sa.total_collateral_usd, 20_000, "I-9: user A collateral correct");
    assert_eq!(sa.total_debt_usd, 10_000, "I-9: user A debt correct");
}

#[test]
fn invariant_user_b_activity_does_not_affect_user_a_summary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);

    client.deposit_collateral_asset(&user_a, &asset, &10_000);
    client.borrow_asset(&user_a, &asset, &5_000);
    let sa_before = client.get_cross_position_summary(&user_a);

    // user_b deposits a large amount into the same asset.
    client.deposit_collateral_asset(&user_b, &asset, &1_000_000);

    let sa_after = client.get_cross_position_summary(&user_a);

    assert_eq!(
        sa_before.total_collateral_usd, sa_after.total_collateral_usd,
        "I-9: isolation"
    );
    assert_eq!(sa_before.total_debt_usd, sa_after.total_debt_usd, "I-9: isolation");
    assert_eq!(sa_before.health_factor, sa_after.health_factor, "I-9: isolation");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-10 — Higher LTV → higher health factor for same amounts
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_higher_ltv_gives_higher_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let collateral = 10_000i128;
    let debt = 3_000i128;

    let ltv_low = 5_000i128;
    let ltv_high = 8_000i128;

    let asset_low = register_asset(&env, &client, ltv_low);
    let asset_high = register_asset(&env, &client, ltv_high);

    let user_low = Address::generate(&env);
    client.deposit_collateral_asset(&user_low, &asset_low, &collateral);
    client.borrow_asset(&user_low, &asset_low, &debt);
    let s_low = client.get_cross_position_summary(&user_low);

    let user_high = Address::generate(&env);
    client.deposit_collateral_asset(&user_high, &asset_high, &collateral);
    client.borrow_asset(&user_high, &asset_high, &debt);
    let s_high = client.get_cross_position_summary(&user_high);

    assert!(
        s_high.health_factor > s_low.health_factor,
        "I-10: LTV {ltv_high} HF ({}) must exceed LTV {ltv_low} HF ({})",
        s_high.health_factor,
        s_low.health_factor,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// I-11 — More collateral → higher HF (monotone)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_more_collateral_increases_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let debt = 1_000i128;
    let mut prev_hf = 0i128;

    for &collateral in &[2_000i128, 5_000, 10_000, 50_000] {
        let user = Address::generate(&env);
        client.deposit_collateral_asset(&user, &asset, &collateral);
        client.borrow_asset(&user, &asset, &debt);

        let s = client.get_cross_position_summary(&user);
        assert!(
            s.health_factor > prev_hf,
            "I-11: collateral {collateral} must give HF ({}) > previous HF ({})",
            s.health_factor,
            prev_hf,
        );
        prev_hf = s.health_factor;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I-12 — More debt → lower HF (monotone)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_more_debt_decreases_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 9_000);
    let collateral = 100_000i128;

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &collateral);

    let mut prev_hf = i128::MAX;

    for &extra_debt in &[1_000i128, 5_000, 20_000, 50_000] {
        client.borrow_asset(&user, &asset, &extra_debt);
        let s = client.get_cross_position_summary(&user);

        assert!(
            s.health_factor < prev_hf,
            "I-12: cumulative debt must lower HF; prev={prev_hf}, new={}",
            s.health_factor,
        );
        prev_hf = s.health_factor;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I-13 — Deposit ordering does not affect totals
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_deposit_order_does_not_affect_total_collateral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let ltv = 7_500i128;
    let a1 = register_asset(&env, &client, ltv);
    let a2 = register_asset(&env, &client, ltv);
    let a3 = register_asset(&env, &client, ltv);

    let (amt1, amt2, amt3) = (3_000i128, 7_000, 11_000);

    // User Alpha: deposit in order a1 → a2 → a3
    let user_alpha = Address::generate(&env);
    client.deposit_collateral_asset(&user_alpha, &a1, &amt1);
    client.deposit_collateral_asset(&user_alpha, &a2, &amt2);
    client.deposit_collateral_asset(&user_alpha, &a3, &amt3);

    // User Beta: deposit in order a3 → a1 → a2
    let user_beta = Address::generate(&env);
    client.deposit_collateral_asset(&user_beta, &a3, &amt3);
    client.deposit_collateral_asset(&user_beta, &a1, &amt1);
    client.deposit_collateral_asset(&user_beta, &a2, &amt2);

    let sa = client.get_cross_position_summary(&user_alpha);
    let sb = client.get_cross_position_summary(&user_beta);

    assert_eq!(
        sa.total_collateral_usd, sb.total_collateral_usd,
        "I-13: order invariant on collateral"
    );
    assert_eq!(sa.health_factor, sb.health_factor, "I-13: order invariant on HF");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-14 — LTV rounding truncates toward zero (floor)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_ltv_rounding_truncates_toward_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // LTV = 3333 bps (33.33 %).
    // For amount = 1: weighted = 1 * 3333 / 10000 = 0 (floor).
    // With weighted = 0 the position cannot support any debt.
    let ltv: i128 = 3_333;
    let asset_col = register_asset(&env, &client, ltv);
    let asset_debt = register_asset(&env, &client, ltv);

    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset_col, &1);

    // Borrow must be rejected because weighted_collateral rounds to 0.
    let result = client.try_borrow_asset(&user, &asset_debt, &1);
    assert_eq!(
        result,
        Err(Ok(CrossAssetError::InsufficientCollateral)),
        "I-14: floor rounding blocks borrow when weighted_collateral = 0"
    );

    // With a larger amount, the floor result is predictable.
    let large_col = 10_007i128;
    let user2 = Address::generate(&env);
    client.deposit_collateral_asset(&user2, &asset_col, &large_col);
    client.borrow_asset(&user2, &asset_debt, &1_000);

    let s2 = client.get_cross_position_summary(&user2);
    let expected_w = usd_value(large_col) * ltv / BPS_SCALE;
    let exp_hf = expected_hf(expected_w, usd_value(1_000));
    assert_eq!(s2.health_factor, exp_hf, "I-14: floor division HF");
}

// ─────────────────────────────────────────────────────────────────────────────
// I-15 — Multi-asset collateral and debt sum independently
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_multi_collateral_multi_debt_sum_independently() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let (ltv_a, ltv_b, ltv_c) = (8_000i128, 7_000, 5_000);
    let ca = register_asset(&env, &client, ltv_a);
    let cb = register_asset(&env, &client, ltv_b);
    let cc = register_asset(&env, &client, ltv_c);

    let user = Address::generate(&env);
    let (amt_a, amt_b, amt_c) = (20_000i128, 15_000, 10_000);
    client.deposit_collateral_asset(&user, &ca, &amt_a);
    client.deposit_collateral_asset(&user, &cb, &amt_b);
    client.deposit_collateral_asset(&user, &cc, &amt_c);

    let (d_a, d_b) = (5_000i128, 4_000);
    client.borrow_asset(&user, &ca, &d_a);
    client.borrow_asset(&user, &cb, &d_b);

    let s = client.get_cross_position_summary(&user);

    let expected_collateral = usd_value(amt_a) + usd_value(amt_b) + usd_value(amt_c);
    let expected_debt = usd_value(d_a) + usd_value(d_b);
    let expected_weighted =
        weighted(amt_a, ltv_a) + weighted(amt_b, ltv_b) + weighted(amt_c, ltv_c);
    let exp_hf = expected_hf(expected_weighted, expected_debt);

    assert_eq!(s.total_collateral_usd, expected_collateral, "I-15: collateral sum");
    assert_eq!(s.total_debt_usd, expected_debt, "I-15: debt sum");
    assert_eq!(s.health_factor, exp_hf, "I-15: HF from totals");
}

// ─────────────────────────────────────────────────────────────────────────────
// Table-driven scenarios (parameterized permutations)
// ─────────────────────────────────────────────────────────────────────────────

/// Run a single scenario: register assets, deposit, borrow, return summary.
///
/// `collateral_configs` is `(amount, ltv)` per collateral asset.
/// `debt_amounts` is the borrow amount for each corresponding asset (0 = no borrow).
fn run_scenario(
    env: &Env,
    client: &LendingContractClient<'_>,
    collateral_configs: &[(i128, i128)],
    debt_amounts: &[i128],
) -> crate::cross_asset::PositionSummary {
    let user = Address::generate(env);
    // Up to 5 assets per scenario — use a fixed-size slot array to avoid Vec.
    let mut assets: [Option<Address>; 5] = [None, None, None, None, None];

    for (i, &(amt, ltv)) in collateral_configs.iter().enumerate() {
        let asset = register_asset(env, client, ltv);
        client.deposit_collateral_asset(&user, &asset, &amt);
        assets[i] = Some(asset);
    }
    for (i, &debt) in debt_amounts.iter().enumerate() {
        if debt > 0 {
            client.borrow_asset(&user, assets[i].as_ref().unwrap(), &debt);
        }
    }
    client.get_cross_position_summary(&user)
}

#[test]
fn table_scenario_single_collateral_no_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let s = run_scenario(&env, &client, &[(10_000, 7_500)], &[0]);
    assert_eq!(s.total_collateral_usd, 10_000);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

#[test]
fn table_scenario_single_asset_at_exact_capacity() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // LTV 75 %, collateral 10 000 → max borrow 7 500 → HF = 10 000
    let s = run_scenario(&env, &client, &[(10_000, 7_500)], &[7_500]);
    assert_eq!(s.total_collateral_usd, 10_000);
    assert_eq!(s.total_debt_usd, 7_500);
    assert_eq!(s.health_factor, BPS_SCALE, "HF at exact capacity = 1.0");
}

#[test]
fn table_scenario_single_asset_healthy_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // LTV 80 %, collateral 10 000, debt 5 000 → HF = 8000*10000/5000 = 16 000
    let s = run_scenario(&env, &client, &[(10_000, 8_000)], &[5_000]);
    assert_eq!(s.total_collateral_usd, 10_000);
    assert_eq!(s.total_debt_usd, 5_000);
    assert_eq!(s.health_factor, 16_000);
}

#[test]
fn table_scenario_two_collateral_two_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // Collateral A: 10 000 @ LTV 75 % → weighted = 7 500
    // Collateral B:  5 000 @ LTV 60 % → weighted = 3 000
    // total_weighted = 10 500
    // Debt A: 3 000; Debt B: 2 000 → total_debt = 5 000
    // HF = 10 500 * 10 000 / 5 000 = 21 000
    let s = run_scenario(
        &env,
        &client,
        &[(10_000, 7_500), (5_000, 6_000)],
        &[3_000, 2_000],
    );
    assert_eq!(s.total_collateral_usd, 15_000);
    assert_eq!(s.total_debt_usd, 5_000);
    assert_eq!(s.health_factor, 21_000);
}

#[test]
fn table_scenario_large_amounts_floor_rounding() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    // Two collateral assets with different LTVs, two debt assets.
    // LTV 90 %, collateral 100 000 → weighted = 90 000
    // LTV 50 %, collateral  50 000 → weighted = 25 000
    // total_weighted = 115 000
    // debt 50 000 + 10 000 = 60 000
    // HF = 115 000 * 10 000 / 60 000 = 19 166 (floor of 19 166.6̄)
    let s = run_scenario(
        &env,
        &client,
        &[(100_000, 9_000), (50_000, 5_000)],
        &[50_000, 10_000],
    );
    assert_eq!(s.total_collateral_usd, 150_000);
    assert_eq!(s.total_debt_usd, 60_000);
    assert_eq!(s.health_factor, 19_166);
}

// ─────────────────────────────────────────────────────────────────────────────
// Security: views do not mutate storage
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn security_view_does_not_mutate_balances() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);

    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);

    let before = client.get_cross_position_summary(&user);

    // Repeatedly read the summary.
    for _ in 0..10 {
        let _ = client.get_cross_position_summary(&user);
    }

    let after = client.get_cross_position_summary(&user);

    assert_eq!(
        before.total_collateral_usd, after.total_collateral_usd,
        "security: view is read-only (collateral)"
    );
    assert_eq!(
        before.total_debt_usd, after.total_debt_usd,
        "security: view is read-only (debt)"
    );
    assert_eq!(
        before.health_factor, after.health_factor,
        "security: view is read-only (HF)"
    );
}

#[test]
fn security_view_on_unknown_user_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let user = Address::generate(&env);
    let s1 = client.get_cross_position_summary(&user);
    let s2 = client.get_cross_position_summary(&user);

    assert_eq!(s1.total_collateral_usd, 0);
    assert_eq!(s1.total_debt_usd, 0);
    assert_eq!(s1.health_factor, HF_NO_DEBT);
    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd);
    assert_eq!(s1.total_debt_usd, s2.total_debt_usd);
    assert_eq!(s1.health_factor, s2.health_factor);
}
