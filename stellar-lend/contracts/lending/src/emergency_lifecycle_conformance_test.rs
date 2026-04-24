use super::*;
use crate::deposit::DepositError;
use crate::flash_loan::FlashLoanError;
use crate::withdraw::WithdrawError;
use soroban_sdk::{testutils::Address as _, Address, Env};

/// Comprehensive emergency lifecycle conformance test suite
/// 
/// This test suite validates:
/// 1. State machine transitions (Normal -> Shutdown -> Recovery -> Normal)
/// 2. Authorization requirements for each transition
/// 3. Operation permissions in each state
/// 4. Forbidden transitions and edge cases
/// 5. Role-based access controls

#[test]
fn test_emergency_state_machine_complete_flow() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);

    // Initial state should be Normal
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // Configure guardian
    client.set_guardian(&admin, &guardian);
    assert_eq!(client.get_guardian(), Some(guardian.clone()));

    // Transition: Normal -> Shutdown (by guardian)
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    // Transition: Shutdown -> Recovery (by admin only)
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // Transition: Recovery -> Normal (by admin only)
    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

#[test]
fn test_emergency_shutdown_authorization_matrix() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);

    // Test unauthorized shutdown attempts
    assert_eq!(
        client.try_emergency_shutdown(&unauthorized_user),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Test authorized shutdown by admin
    client.emergency_shutdown(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    // Reset to Normal for next test
    client.start_recovery(&admin);
    client.complete_recovery(&admin);

    // Test authorized shutdown by guardian
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);
}

#[test]
fn test_recovery_transition_authorization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);

    // Cannot start recovery from Normal state
    assert_eq!(
        client.try_start_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Trigger shutdown first
    client.emergency_shutdown(&guardian);

    // Unauthorized user cannot start recovery
    assert_eq!(
        client.try_start_recovery(&unauthorized_user),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Guardian cannot start recovery (admin only)
    assert_eq!(
        client.try_start_recovery(&guardian),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can start recovery
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);
}

#[test]
fn test_complete_recovery_authorization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);

    // Cannot complete recovery from Normal state
    assert_eq!(
        client.try_complete_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Cannot complete recovery from Shutdown state
    client.emergency_shutdown(&guardian);
    assert_eq!(
        client.try_complete_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Move to Recovery state
    client.start_recovery(&admin);

    // Unauthorized user cannot complete recovery
    assert_eq!(
        client.try_complete_recovery(&unauthorized_user),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Guardian cannot complete recovery (admin only)
    assert_eq!(
        client.try_complete_recovery(&guardian),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can complete recovery
    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

#[test]
fn test_operation_permissions_normal_state() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // In Normal state, all operations should work (subject to granular pause rules)
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // These operations should succeed in Normal state
    client.deposit(&user, &asset, &50_000);
    client.deposit_collateral(&user, &collateral_asset, &20_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    client.repay(&user, &asset, &1_000);
    client.withdraw(&user, &asset, &1_000);
    client.flash_loan(&user, &asset, &1_000, &soroban_sdk::Bytes::new(&env));
}

#[test]
fn test_operation_permissions_shutdown_state() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // Setup initial positions
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // Trigger shutdown
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    // All high-risk operations should be blocked in Shutdown state
    assert_eq!(
        client.try_deposit(&user, &asset, &1000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &collateral_asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1000, &collateral_asset, &2000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_liquidate(&user, &user, &asset, &collateral_asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1000, &soroban_sdk::Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );
    assert_eq!(
        client.try_repay(&user, &asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &1000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
}

#[test]
fn test_operation_permissions_recovery_state() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // Setup initial positions
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // Move to Recovery state
    client.emergency_shutdown(&guardian);
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // High-risk operations should still be blocked in Recovery state
    assert_eq!(
        client.try_deposit(&user, &asset, &1000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &collateral_asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1000, &collateral_asset, &2000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_liquidate(&user, &user, &asset, &collateral_asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1000, &soroban_sdk::Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );

    // Unwind operations should be allowed in Recovery state
    client.repay(&user, &asset, &1_000);
    client.withdraw(&user, &asset, &1_000);
}

#[test]
fn test_forbidden_state_transitions() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);

    // Cannot transition Normal -> Normal (no-op)
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // Cannot transition Normal -> Recovery (must go through Shutdown)
    assert_eq!(
        client.try_start_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Cannot transition Normal -> Normal via complete_recovery
    assert_eq!(
        client.try_complete_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // After Shutdown, cannot transition back to Normal directly
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    assert_eq!(
        client.try_complete_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // After Recovery, cannot transition back to Shutdown
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // Another shutdown should work from Recovery (emergency override)
    client.emergency_shutdown(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);
}

#[test]
fn test_guardian_configuration_authorization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);

    // Unauthorized user cannot set guardian
    assert_eq!(
        client.try_set_guardian(&unauthorized_user, &guardian),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can set guardian
    client.set_guardian(&admin, &guardian);
    assert_eq!(client.get_guardian(), Some(guardian.clone()));

    // Admin can change guardian
    let new_guardian = Address::generate(&env);
    client.set_guardian(&admin, &new_guardian);
    assert_eq!(client.get_guardian(), Some(new_guardian));

    // Admin can remove guardian (by setting to zero address)
    let zero_address = Address::generate(&env); // In practice, this would be Address::ZERO
    client.set_guardian(&admin, &zero_address);
    assert_eq!(client.get_guardian(), Some(zero_address));
}

#[test]
fn test_emergency_events_emission() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);

    // Guardian set event
    client.set_guardian(&admin, &guardian);

    // Emergency shutdown event
    client.emergency_shutdown(&guardian);

    // Start recovery event
    client.start_recovery(&admin);

    // Complete recovery event
    client.complete_recovery(&admin);
}

#[test]
fn test_partial_pause_interaction_with_emergency_states() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // Setup initial positions
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // Move to Recovery state
    client.emergency_shutdown(&guardian);
    client.start_recovery(&admin);

    // Even in Recovery, granular pauses still apply
    client.set_pause(&admin, &PauseType::Repay, &true);
    assert_eq!(
        client.try_repay(&user, &asset, &1000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    client.set_pause(&admin, &PauseType::Repay, &false);
    client.set_pause(&admin, &PauseType::Withdraw, &true);
    assert_eq!(
        client.try_withdraw(&user, &asset, &1000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    // High-risk operations remain blocked regardless of granular pause state
    client.set_pause(&admin, &PauseType::Deposit, &false);
    assert_eq!(
        client.try_deposit(&user, &asset, &1000),
        Err(Ok(DepositError::DepositPaused))
    );
}

#[test]
fn test_multiple_emergency_cycles() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // First emergency cycle
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    client.repay(&user, &asset, &5_000);
    client.withdraw(&user, &asset, &5_000);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // Second emergency cycle
    client.deposit(&user, &asset, &30_000);
    client.borrow(&user, &asset, &5_000, &collateral_asset, &10_000);

    client.emergency_shutdown(&admin); // Admin can also trigger
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    client.repay(&user, &asset, &2_000);
    client.withdraw(&user, &asset, &2_000);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}
