#![cfg(test)]

use crate::{
    borrow::BorrowError,
    deposit::{AssetParams, DepositDataKey, DepositError, Position},
    reentrancy::{is_locked, ReentrancyGuard, REENTRANCY_ERROR_CODE},
    repay::RepayError,
    withdraw::WithdrawError,
    HelloContract, HelloContractClient,
};
use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env, Symbol};

#[contract]
pub struct MaliciousToken;

#[contractimpl]
impl MaliciousToken {
    pub fn balance(_env: Env, _id: Address) -> i128 {
        1_000_000
    }

    pub fn transfer_from(env: Env, _spender: Address, from: Address, _to: Address, _amount: i128) {
        attempt_callback_reentry(&env, &from);
    }

    pub fn transfer(env: Env, _from: Address, to: Address, _amount: i128) {
        attempt_callback_reentry(&env, &to);
    }
}

fn attempt_callback_reentry(env: &Env, user: &Address) {
    let target_key = Symbol::new(env, "HELLO_TARGET");
    let target = env
        .storage()
        .persistent()
        .get::<Symbol, Address>(&target_key)
        .expect("target contract must be configured");

    let client = HelloContractClient::new(env, &target);
    let token = Some(env.current_contract_address());

    let deposit_result = client.try_deposit_collateral(user, &token, &100);
    assert!(deposit_result.is_err());

    let withdraw_result = client.try_withdraw_collateral(user, &token, &100);
    assert!(withdraw_result.is_err());

    let borrow_result = client.try_borrow_asset(user, &token, &100);
    assert!(borrow_result.is_err());

    let repay_result = client.try_repay_debt(user, &token, &100);
    assert!(repay_result.is_err());
}

fn setup_test() -> (Env, Address, HelloContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    client.initialize(&admin).unwrap();

    let malicious_token_id = env.register(MaliciousToken, ());

    env.as_contract(&malicious_token_id, || {
        env.storage()
            .persistent()
            .set(&Symbol::new(&env, "HELLO_TARGET"), &contract_id);
    });

    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &DepositDataKey::AssetParams(malicious_token_id.clone()),
            &AssetParams {
                deposit_enabled: true,
                collateral_factor: 10_000,
                max_deposit: 10_000_000,
                borrow_fee_bps: 0,
            },
        );
    });

    let static_client = unsafe {
        core::mem::transmute::<HelloContractClient<'_>, HelloContractClient<'static>>(client)
    };

    (env, contract_id, static_client, malicious_token_id, user)
}

fn seed_position(env: &Env, contract_id: &Address, user: &Address, collateral: i128, debt: i128) {
    env.as_contract(contract_id, || {
        env.storage().persistent().set(
            &DepositDataKey::CollateralBalance(user.clone()),
            &collateral,
        );
        env.storage().persistent().set(
            &DepositDataKey::Position(user.clone()),
            &Position {
                collateral,
                debt,
                borrow_interest: 0,
                last_accrual_time: env.ledger().timestamp(),
            },
        );
    });
}

#[test]
fn reentrancy_guard_rejects_nested_entry_and_unlocks_after_drop() {
    let env = Env::default();
    let contract_id = env.register(HelloContract, ());

    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));

        let guard = ReentrancyGuard::new(&env).unwrap();
        assert!(is_locked(&env));

        assert_eq!(
            ReentrancyGuard::new(&env).unwrap_err(),
            REENTRANCY_ERROR_CODE
        );

        drop(guard);

        assert!(!is_locked(&env));
        assert!(ReentrancyGuard::new(&env).is_ok());
    });
}

#[test]
fn deposit_rejects_callback_reentry_and_releases_lock() {
    let (env, contract_id, client, token_id, user) = setup_test();

    client.deposit_collateral(&user, &Some(token_id), &1_000).unwrap();

    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_rejects_callback_reentry_and_releases_lock() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    client.withdraw_collateral(&user, &Some(token_id), &500).unwrap();

    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_rejects_callback_reentry_and_releases_lock() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    client.repay_debt(&user, &Some(token_id), &500).unwrap();

    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

// --------------------------------------------------------------------------- 
// Extended reentrancy regression tests for repay and withdraw
// ---------------------------------------------------------------------------

#[test]
fn repay_reentrancy_with_zero_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Zero amount should fail before reentrancy check
    let result = client.try_repay_debt(&user, &Some(token_id), &0);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_with_negative_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Negative amount should fail before reentrancy check
    let result = client.try_repay_debt(&user, &Some(token_id), &-100);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_when_no_debt() {
    let (env, contract_id, client, token_id, user) = setup_test();
    // Don't seed any debt

    // Should fail before reentrancy check due to no debt
    let result = client.try_repay_debt(&user, &Some(token_id), &100);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_with_max_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Use maximum possible amount
    let result = client.repay_debt(&user, &Some(token_id), &i128::MAX);
    assert!(result.is_ok()); // Should succeed and repay all debt
    
    // Verify lock is released
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_zero_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Zero amount should fail before reentrancy check
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &0);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_negative_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Negative amount should fail before reentrancy check
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &-100);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_insufficient_collateral() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 500, 0);

    // Try to withdraw more than available
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &1_000);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_undercollateralized_position() {
    let (env, contract_id, client, token_id, user) = setup_test();
    // Create a position that would become undercollateralized
    seed_position(&env, &contract_id, &user, 1_000, 800); // High debt ratio

    // Try to withdraw - should fail due to health check before reentrancy
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &100);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_max_amount() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 0);

    // Use maximum possible amount - should fail due to overflow or insufficient balance
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &i128::MAX);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_during_token_transfer_callback() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    client.initialize(&admin).unwrap();

    // Create a malicious token that calls repay during transfer
    let malicious_token_id = env.register(MaliciousToken, ());

    env.as_contract(&malicious_token_id, || {
        env.storage()
            .persistent()
            .set(&Symbol::new(&env, "HELLO_TARGET"), &contract_id);
    });

    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &DepositDataKey::AssetParams(malicious_token_id.clone()),
            &AssetParams {
                deposit_enabled: true,
                collateral_factor: 10_000,
                max_deposit: 10_000_000,
                borrow_fee_bps: 0,
            },
        );
    });

    // Seed position with debt
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Attempt repay - malicious token callback should be rejected
    let result = client.repay_debt(&user, &Some(malicious_token_id), &500);
    assert!(result.is_ok()); // Original call succeeds
    
    // Verify lock is released after successful operation
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_during_token_transfer_callback() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(&env, &contract_id);
    client.initialize(&admin).unwrap();

    // Create a malicious token that calls withdraw during transfer
    let malicious_token_id = env.register(MaliciousToken, ());

    env.as_contract(&malicious_token_id, || {
        env.storage()
            .persistent()
            .set(&Symbol::new(&env, "HELLO_TARGET"), &contract_id);
    });

    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &DepositDataKey::AssetParams(malicious_token_id.clone()),
            &AssetParams {
                deposit_enabled: true,
                collateral_factor: 10_000,
                max_deposit: 10_000_000,
                borrow_fee_bps: 0,
            },
        );
    });

    // Seed position with collateral
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Attempt withdraw - malicious token callback should be rejected
    let result = client.withdraw_collateral(&user, &Some(malicious_token_id), &500);
    assert!(result.is_ok()); // Original call succeeds
    
    // Verify lock is released after successful operation
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_with_paused_operation() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Pause repay operations
    env.as_contract(&contract_id, || {
        let mut pause_map = Map::new(&env);
        pause_map.set(Symbol::new(&env, "pause_repay"), true);
        env.storage()
            .persistent()
            .set(&DepositDataKey::PauseSwitches, &pause_map);
    });

    // Should fail before reentrancy check due to pause
    let result = client.try_repay_debt(&user, &Some(token_id), &500);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_with_paused_operation() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Pause withdraw operations
    env.as_contract(&contract_id, || {
        let mut pause_map = Map::new(&env);
        pause_map.set(Symbol::new(&env, "pause_withdraw"), true);
        env.storage()
            .persistent()
            .set(&DepositDataKey::PauseSwitches, &pause_map);
    });

    // Should fail before reentrancy check due to pause
    let result = client.try_withdraw_collateral(&user, &Some(token_id), &500);
    assert!(result.is_err());
    
    // Verify lock is not set
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_multiple_concurrent_attempts() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Start first repay operation
    env.as_contract(&contract_id, || {
        let _guard = ReentrancyGuard::new(&env).unwrap();
        
        // Attempt second repay operation - should fail
        let repay_result = crate::repay::repay_debt(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(repay_result, Err(RepayError::Reentrancy));
    });

    // Lock should be released
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_multiple_concurrent_attempts() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Start first withdraw operation
    env.as_contract(&contract_id, || {
        let _guard = ReentrancyGuard::new(&env).unwrap();
        
        // Attempt second withdraw operation - should fail
        let withdraw_result = crate::withdraw::withdraw_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(withdraw_result, Err(WithdrawError::Reentrancy));
    });

    // Lock should be released
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn repay_reentrancy_cross_operation_blocking() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    // Start repay operation
    env.as_contract(&contract_id, || {
        let _guard = ReentrancyGuard::new(&env).unwrap();
        
        // Attempt withdraw operation - should fail due to reentrancy guard
        let withdraw_result = crate::withdraw::withdraw_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(withdraw_result, Err(WithdrawError::Reentrancy));
        
        // Attempt borrow operation - should fail due to reentrancy guard
        let borrow_result = crate::borrow::borrow_asset(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(borrow_result, Err(BorrowError::Reentrancy));
        
        // Attempt deposit operation - should fail due to reentrancy guard
        let deposit_result = crate::deposit::deposit_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(deposit_result, Err(DepositError::Reentrancy));
    });

    // Lock should be released
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn withdraw_reentrancy_cross_operation_blocking() {
    let (env, contract_id, client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 1_000, 0);

    // Start withdraw operation
    env.as_contract(&contract_id, || {
        let _guard = ReentrancyGuard::new(&env).unwrap();
        
        // Attempt repay operation - should fail due to reentrancy guard
        let repay_result = crate::repay::repay_debt(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(repay_result, Err(RepayError::Reentrancy));
        
        // Attempt borrow operation - should fail due to reentrancy guard
        let borrow_result = crate::borrow::borrow_asset(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(borrow_result, Err(BorrowError::Reentrancy));
        
        // Attempt deposit operation - should fail due to reentrancy guard
        let deposit_result = crate::deposit::deposit_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(deposit_result, Err(DepositError::Reentrancy));
    });

    // Lock should be released
    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}

#[test]
fn protected_entrypoints_map_preexisting_lock_to_operation_errors() {
    let (env, contract_id, _client, token_id, user) = setup_test();
    seed_position(&env, &contract_id, &user, 10_000, 1_000);

    env.as_contract(&contract_id, || {
        let _guard = ReentrancyGuard::new(&env).unwrap();

        let deposit_result =
            crate::deposit::deposit_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(deposit_result, Err(DepositError::Reentrancy));

        let withdraw_result =
            crate::withdraw::withdraw_collateral(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(withdraw_result, Err(WithdrawError::Reentrancy));

        let borrow_result =
            crate::borrow::borrow_asset(&env, user.clone(), Some(token_id.clone()), 100);
        assert_eq!(borrow_result, Err(BorrowError::Reentrancy));

        let repay_result = crate::repay::repay_debt(&env, user.clone(), Some(token_id), 100);
        assert_eq!(repay_result, Err(RepayError::Reentrancy));
    });

    env.as_contract(&contract_id, || {
        assert!(!is_locked(&env));
    });
}
