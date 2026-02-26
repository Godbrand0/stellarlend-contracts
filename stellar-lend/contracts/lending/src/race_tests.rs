//! # Intra-Ledger-Block Operation Ordering and Race Tests
//!
//! These tests simulate sequences of operations within a single ledger context.
//! In Soroban, consistent results must be maintained regardless of call ordering
//! where intended (e.g. net effect of deposits and withdrawals).
//!
//! ## Ordering Assumptions
//! 1. **Balance Invariance**: The final balance should be the same regardless of order,
//!    provided no intermediate state violates protocol rules (e.g. withdrawing more than available).
//! 2. **Zero Interest within Ledger**: Since the ledger timestamp is constant within a block,
//!    operations like borrow/repay do not accrue interest intra-ledger.
//! 3. **Auth Requirements**: Each operation still requires proper authorization.
//!
//! ## Security Guarantees
//! - Atomic state updates ensure no partial executions.
//! - Protocol invariants (like collateral ratios) are checked at each step.

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup_race_test(
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
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    (client, admin, user, asset, collateral_asset)
}

#[test]
fn test_intra_block_deposit_withdraw_same_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, _collateral_asset) = setup_race_test(&env);

    // Sequence: Deposit 10,000 then Withdraw 10,000 in same ledger
    client.deposit(&user, &asset, &10_000);
    client.withdraw(&user, &asset, &10_000);

    let position = client.get_user_collateral_deposit(&user, &asset);
    assert_eq!(position.amount, 0);
}

#[test]
fn test_intra_block_borrow_repay() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup_race_test(&env);

    // Initial deposit for collateral
    client.deposit(&user, &collateral_asset, &50_000);

    // Sequence: Borrow 10,000 then Repay 5,000
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    client.repay(&user, &asset, &5_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 5_000);
}

#[test]
fn test_intra_block_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup_race_test(&env);

    client.deposit(&user, &collateral_asset, &100_000);
    client.borrow(&user, &asset, &20_000, &collateral_asset, &40_000);
    client.repay(&user, &asset, &20_000);
    client.withdraw(&user, &collateral_asset, &50_000);

    let pos_dep = client.get_user_collateral_deposit(&user, &collateral_asset);
    assert_eq!(pos_dep.amount, 50_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 0);
}

#[test]
fn test_intra_block_multi_user_interaction() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user1, asset, _collateral_asset) = setup_race_test(&env);
    let user2 = Address::generate(&env);

    client.deposit(&user1, &asset, &10_000);
    client.deposit(&user2, &asset, &20_000);
    client.withdraw(&user1, &asset, &5_000);
    client.withdraw(&user2, &asset, &10_000);

    let pos1 = client.get_user_collateral_deposit(&user1, &asset);
    let pos2 = client.get_user_collateral_deposit(&user2, &asset);

    assert_eq!(pos1.amount, 5_000);
    assert_eq!(pos2.amount, 10_000);
}

#[test]
fn test_intra_block_invalid_ordering_withdraw_first() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, _collateral_asset) = setup_race_test(&env);

    client.deposit(&user, &asset, &10_000);

    let result = client.try_withdraw(&user, &asset, &15_000);
    assert!(result.is_err());

    client.deposit(&user, &asset, &10_000);

    let pos = client.get_user_collateral_deposit(&user, &asset);
    assert_eq!(pos.amount, 20_000);
}

#[test]
fn test_intra_block_excessive_borrow_repay_race() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup_race_test(&env);

    client.deposit(&user, &collateral_asset, &1_000_000);

    for i in 1..=5 {
        client.borrow(&user, &asset, &(i * 1000), &collateral_asset, &(i * 2000));
        client.repay(&user, &asset, &(i * 500));
    }

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 7_500);
}
