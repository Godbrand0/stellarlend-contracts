use crate::borrow::BorrowCollateral;
use crate::borrow::{calculate_interest, validate_collateral_ratio, DebtPosition};
use crate::views::{collateral_value, compute_health_factor, HEALTH_FACTOR_NO_DEBT};
use crate::LendingContract;
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, Address, Env};

#[test]
fn test_interest_calculation_extreme_values() {
    let env = Env::default();

    // Test with maximum principal and maximum time
    let position = DebtPosition {
        borrowed_amount: i128::MAX,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    // Set ledger time to far future (100 years from now)
    env.ledger().with_mut(|li| li.timestamp = 100 * 31536000);

    // calculate_interest uses I256 intermediate, so it handles large results
    let interest = calculate_interest(&env, &position);
    assert!(interest > 0);

    // Test with large amount (10^30) and 3 years (approx 10^8 seconds)
    // Intermediate: 10^30 * 500 * 10^8 = 5 * 10^40 (overflows i128)
    // Result: ~1.5 * 10^29 (fits in i128)
    let large_position = DebtPosition {
        borrowed_amount: 1_000_000_000_000_000_000_000_000_000_000i128,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };
    env.ledger().with_mut(|li| li.timestamp = 3 * 31536000);

    let large_interest = calculate_interest(&env, &large_position);
    // 10^30 * 0.05 * 3 = 1.5 * 10^29
    assert!(large_interest > 100_000_000_000_000_000_000_000_000_000i128); // > 10^29
    assert!(large_interest < 200_000_000_000_000_000_000_000_000_000i128); // < 2*10^29
}

#[test]
fn test_collateral_ratio_overflow() {
    // i128::MAX borrow should trigger overflow error in validate_collateral_ratio
    let result = validate_collateral_ratio(100, i128::MAX);
    assert!(result.is_err());
}

#[test]
fn test_views_math_safety() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());

    env.as_contract(&contract_id, || {
        // Now storage is accessible
        let collateral = BorrowCollateral {
            amount: i128::MAX,
            asset: Address::generate(&env),
        };

        // Should return 0 if no oracle
        assert_eq!(collateral_value(&env, &collateral), 0);

        // Health factor math bounds
        let cv = i128::MAX / 2;
        let dv = 1;
        // This would overflow (cv * 8000 / 10000) * 10000 / 1 -> returns 0 on overflow
        let hf = compute_health_factor(&env, cv, dv, true);
        assert_eq!(hf, 0);

        // Zero debt health factor
        assert_eq!(
            compute_health_factor(&env, 1000, 0, false),
            HEALTH_FACTOR_NO_DEBT
        );
    });
}
