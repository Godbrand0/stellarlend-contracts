use crate::borrow::BorrowCollateral;
use crate::borrow::{calculate_interest, validate_collateral_ratio, BorrowDataKey, DebtPosition};
use crate::views::{collateral_value, compute_health_factor, HEALTH_FACTOR_NO_DEBT};
use crate::LendingContract;
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, Address, Env};

#[test]
fn test_interest_monotonic_for_large_ledger_jumps() {
    let env = Env::default();
    let position = DebtPosition {
        borrowed_amount: 1_000_000,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    let checkpoints = [1u64, 10u64, 100u64, 500u64];
    let mut previous_interest = 0i128;

    for years in checkpoints {
        env.ledger()
            .with_mut(|li| li.timestamp = years * 31_536_000);
        let interest = calculate_interest(&env, &position);
        assert!(interest >= previous_interest);

        let upper_bound = position
            .borrowed_amount
            .checked_mul(5)
            .and_then(|v| v.checked_mul(years as i128))
            .and_then(|v| v.checked_div(100))
            .unwrap();
        assert!(interest <= upper_bound);

        previous_interest = interest;
    }
}

#[test]
fn test_interest_saturates_to_i128_max_at_extreme_horizon() {
    let env = Env::default();
    let position = DebtPosition {
        borrowed_amount: i128::MAX,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    env.ledger().with_mut(|li| li.timestamp = u64::MAX);
    let interest = calculate_interest(&env, &position);
    assert_eq!(interest, i128::MAX);
}

#[test]
fn test_get_user_debt_interest_addition_saturates() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let initial = DebtPosition {
            borrowed_amount: i128::MAX,
            interest_accrued: i128::MAX - 10,
            last_update: 0,
            asset: user.clone(),
        };
        env.storage()
            .persistent()
            .set(&BorrowDataKey::BorrowUserDebt(user.clone()), &initial);
    });

    env.ledger().with_mut(|li| li.timestamp = u64::MAX);
    let debt = env.as_contract(&contract_id, || crate::borrow::get_user_debt(&env, &user));
    assert_eq!(debt.interest_accrued, i128::MAX);
}

#[test]
fn test_interest_calculation_extreme_values() {
    let env = Env::default();

    let position = DebtPosition {
        borrowed_amount: i128::MAX,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    env.ledger().with_mut(|li| li.timestamp = 100 * 31536000);
    let interest = calculate_interest(&env, &position);
    assert!(interest > 0);
}
