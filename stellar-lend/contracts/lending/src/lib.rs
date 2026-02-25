#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, Val, Vec};

mod borrow;
mod deposit;
mod flash_loan;
mod pause;
mod token_receiver;
mod withdraw;

use borrow::{
    borrow as borrow_impl, deposit as borrow_deposit, get_admin as get_protocol_admin,
    get_user_collateral as get_borrow_collateral, get_user_debt as get_user_debt_impl,
    initialize_borrow_settings as init_borrow_settings_impl, repay as borrow_repay,
    set_admin as set_protocol_admin, set_liquidation_threshold_bps as set_liq_threshold_impl,
    set_oracle as set_oracle_impl, BorrowCollateral, BorrowError, DebtPosition,
};
use deposit::{
    deposit as deposit_impl, get_user_collateral as get_deposit_collateral_impl,
    initialize_deposit_settings as init_deposit_settings_impl, DepositCollateral, DepositError,
};
use flash_loan::{
    flash_loan as flash_loan_impl, set_flash_loan_fee_bps as set_flash_loan_fee_impl,
    FlashLoanError,
};
use pause::{is_paused, set_pause as set_pause_impl, PauseType};
use token_receiver::receive as receive_impl;

mod views;
use views::{
    get_collateral_balance, get_collateral_value, get_debt_balance, get_debt_value,
    get_health_factor, get_user_position, UserPositionSummary,
};

mod data_store;
use stellarlend_common::upgrade;
pub use stellarlend_common::upgrade::{UpgradeError, UpgradeStage, UpgradeStatus};
pub use withdraw::WithdrawError;

#[cfg(test)]
mod borrow_test;
#[cfg(test)]
mod deposit_test;
#[cfg(test)]
mod flash_loan_test;
#[cfg(test)]
mod pause_test;
#[cfg(test)]
mod token_receiver_test;
#[cfg(test)]
mod views_test;

#[cfg(test)]
mod data_store_test;
#[cfg(test)]
mod upgrade_test;
#[cfg(test)]
mod withdraw_test;

#[contract]
pub struct LendingContract;

#[contractimpl]
impl LendingContract {
    /// Initialize the protocol with admin and settings
    pub fn initialize(
        env: Env,
        admin: Address,
        debt_ceiling: i128,
        min_borrow_amount: i128,
    ) -> Result<(), BorrowError> {
        if get_protocol_admin(&env).is_some() {
            return Err(BorrowError::Unauthorized);
        }
        set_protocol_admin(&env, &admin);
        init_borrow_settings_impl(&env, debt_ceiling, min_borrow_amount)?;
        Ok(())
    }

    /// Borrow assets against deposited collateral
    pub fn borrow(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
        collateral_asset: Address,
        collateral_amount: i128,
    ) -> Result<(), BorrowError> {
        borrow_impl(
            &env,
            user,
            asset,
            amount,
            collateral_asset,
            collateral_amount,
        )
    }

    /// Set protocol pause state for a specific operation (admin only)
    pub fn set_pause(
        env: Env,
        admin: Address,
        pause_type: PauseType,
        paused: bool,
    ) -> Result<(), BorrowError> {
        let current_admin = get_protocol_admin(&env).ok_or(BorrowError::Unauthorized)?;
        if admin != current_admin {
            return Err(BorrowError::Unauthorized);
        }
        admin.require_auth();
        set_pause_impl(&env, admin, pause_type, paused);
        Ok(())
    }

    /// Repay borrowed assets
    pub fn repay(env: Env, user: Address, asset: Address, amount: i128) -> Result<(), BorrowError> {
        user.require_auth();
        if is_paused(&env, PauseType::Repay) {
            return Err(BorrowError::ProtocolPaused);
        }
        borrow_repay(&env, user, asset, amount)
    }

    /// Deposit collateral for a borrow position
    pub fn deposit_collateral(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
    ) -> Result<(), BorrowError> {
        user.require_auth();
        if is_paused(&env, PauseType::Deposit) {
            return Err(BorrowError::ProtocolPaused);
        }
        borrow_deposit(&env, user, asset, amount)
    }

    /// Deposit collateral into the protocol
    pub fn deposit(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
    ) -> Result<i128, DepositError> {
        if is_paused(&env, PauseType::Deposit) {
            return Err(DepositError::DepositPaused);
        }
        deposit_impl(&env, user, asset, amount)
    }

    /// Liquidate a position
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        _borrower: Address,
        _debt_asset: Address,
        _collateral_asset: Address,
        _amount: i128,
    ) -> Result<(), BorrowError> {
        liquidator.require_auth();
        if is_paused(&env, PauseType::Liquidation) {
            return Err(BorrowError::ProtocolPaused);
        }
        // Stub implementation, or call borrow::liquidate if it exists
        Ok(())
    }

    /// Get user's debt position
    pub fn get_user_debt(env: Env, user: Address) -> DebtPosition {
        get_user_debt_impl(&env, &user)
    }

    /// Get user's collateral position (borrow module)
    pub fn get_user_collateral(env: Env, user: Address) -> BorrowCollateral {
        get_borrow_collateral(&env, &user)
    }

    // ═══════════════════════════════════════════════════════════════════
    // View functions (read-only; for frontends and liquidations)
    // ═══════════════════════════════════════════════════════════════════

    /// Returns the user's collateral balance (raw amount).
    pub fn get_collateral_balance(env: Env, user: Address) -> i128 {
        get_collateral_balance(&env, &user)
    }

    /// Returns the user's debt balance (principal + accrued interest).
    pub fn get_debt_balance(env: Env, user: Address) -> i128 {
        get_debt_balance(&env, &user)
    }

    /// Returns the user's collateral value in common unit (e.g. USD 8 decimals). 0 if oracle not set.
    pub fn get_collateral_value(env: Env, user: Address) -> i128 {
        get_collateral_value(&env, &user)
    }

    /// Returns the user's debt value in common unit. 0 if oracle not set.
    pub fn get_debt_value(env: Env, user: Address) -> i128 {
        get_debt_value(&env, &user)
    }

    /// Returns health factor (scaled 10000 = 1.0). Above 10000 = healthy; below = liquidatable.
    pub fn get_health_factor(env: Env, user: Address) -> i128 {
        get_health_factor(&env, &user)
    }

    /// Returns full position summary: collateral/debt balances and values, and health factor.
    pub fn get_user_position(env: Env, user: Address) -> UserPositionSummary {
        get_user_position(&env, &user)
    }

    /// Set oracle address for price feeds (admin only).
    pub fn set_oracle(env: Env, admin: Address, oracle: Address) -> Result<(), BorrowError> {
        set_oracle_impl(&env, &admin, oracle)
    }

    /// Set liquidation threshold in basis points, e.g. 8000 = 80% (admin only).
    pub fn set_liquidation_threshold_bps(
        env: Env,
        admin: Address,
        bps: i128,
    ) -> Result<(), BorrowError> {
        set_liq_threshold_impl(&env, &admin, bps)
    }

    /// Initialize borrow settings (admin only)
    pub fn initialize_borrow_settings(
        env: Env,
        debt_ceiling: i128,
        min_borrow_amount: i128,
    ) -> Result<(), BorrowError> {
        let current_admin = get_protocol_admin(&env).ok_or(BorrowError::Unauthorized)?;
        current_admin.require_auth();
        init_borrow_settings_impl(&env, debt_ceiling, min_borrow_amount)
    }

    /// Initialize deposit settings (admin only)
    pub fn initialize_deposit_settings(
        env: Env,
        deposit_cap: i128,
        min_deposit_amount: i128,
    ) -> Result<(), DepositError> {
        let current_admin = get_protocol_admin(&env).ok_or(DepositError::Unauthorized)?;
        current_admin.require_auth();
        init_deposit_settings_impl(&env, deposit_cap, min_deposit_amount)
    }

    /// Set deposit pause state (admin only)
    /// Deprecated: use set_pause instead
    pub fn set_deposit_paused(env: Env, paused: bool) -> Result<(), DepositError> {
        let admin = get_protocol_admin(&env).ok_or(DepositError::Unauthorized)?;
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&pause::PauseDataKey::State(PauseType::Deposit), &paused);
        Ok(())
    }

    /// Get user's deposit collateral position
    pub fn get_user_collateral_deposit(
        env: Env,
        user: Address,
        asset: Address,
    ) -> DepositCollateral {
        get_deposit_collateral_impl(&env, &user, &asset)
    }

    /// Get protocol admin
    pub fn get_admin(env: Env) -> Option<Address> {
        get_protocol_admin(&env)
    }

    /// Execute a flash loan
    pub fn flash_loan(
        env: Env,
        receiver: Address,
        asset: Address,
        amount: i128,
        params: Bytes,
    ) -> Result<(), FlashLoanError> {
        flash_loan_impl(&env, receiver, asset, amount, params)
    }

    /// Set the flash loan fee in basis points (admin only)
    pub fn set_flash_loan_fee_bps(env: Env, fee_bps: i128) -> Result<(), FlashLoanError> {
        let current_admin = get_protocol_admin(&env).ok_or(FlashLoanError::Unauthorized)?;
        current_admin.require_auth();
        set_flash_loan_fee_impl(&env, fee_bps)
    }

    /// Withdraw collateral from the protocol
    pub fn withdraw(
        env: Env,
        user: Address,
        asset: Address,
        amount: i128,
    ) -> Result<i128, withdraw::WithdrawError> {
        if is_paused(&env, PauseType::Withdraw) {
            return Err(withdraw::WithdrawError::WithdrawPaused);
        }
        withdraw::withdraw(&env, user, asset, amount)
    }

    /// Initialize withdraw settings (admin only)
    pub fn initialize_withdraw_settings(
        env: Env,
        min_withdraw_amount: i128,
    ) -> Result<(), withdraw::WithdrawError> {
        let current_admin =
            get_protocol_admin(&env).ok_or(withdraw::WithdrawError::Unauthorized)?;
        current_admin.require_auth();
        withdraw::initialize_withdraw_settings(&env, min_withdraw_amount)
    }

    /// Set withdraw pause state (admin only)
    pub fn set_withdraw_paused(env: Env, paused: bool) -> Result<(), withdraw::WithdrawError> {
        let admin = get_protocol_admin(&env).ok_or(withdraw::WithdrawError::Unauthorized)?;
        admin.require_auth();
        withdraw::set_withdraw_paused(&env, paused)
    }

    /// Token receiver hook
    pub fn receive(
        env: Env,
        token_asset: Address,
        from: Address,
        amount: i128,
        payload: Vec<Val>,
    ) -> Result<(), BorrowError> {
        receive_impl(env, token_asset, from, amount, payload)
    }

    // ───────────────────────────────────────────────────
    // Upgrade Management (Governance)
    // ───────────────────────────────────────────────────

    pub fn upgrade_init(
        env: Env,
        admin: Address,
        current_wasm_hash: BytesN<32>,
        required_approvals: u32,
    ) {
        upgrade::UpgradeManager::init(env, admin, current_wasm_hash, required_approvals);
    }

    pub fn upgrade_add_approver(env: Env, caller: Address, approver: Address) {
        upgrade::UpgradeManager::add_approver(env, caller, approver);
    }

    pub fn upgrade_propose(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) -> u64 {
        upgrade::UpgradeManager::upgrade_propose(env, caller, new_wasm_hash, new_version)
    }

    pub fn upgrade_approve(env: Env, caller: Address, proposal_id: u64) -> u32 {
        upgrade::UpgradeManager::upgrade_approve(env, caller, proposal_id)
    }

    pub fn upgrade_execute(env: Env, caller: Address, proposal_id: u64) {
        upgrade::UpgradeManager::upgrade_execute(env, caller, proposal_id);
    }

    pub fn upgrade_rollback(env: Env, caller: Address, proposal_id: u64) {
        upgrade::UpgradeManager::upgrade_rollback(env, caller, proposal_id);
    }

    pub fn upgrade_status(env: Env, proposal_id: u64) -> upgrade::UpgradeStatus {
        upgrade::UpgradeManager::upgrade_status(env, proposal_id)
    }

    pub fn current_wasm_hash(env: Env) -> BytesN<32> {
        upgrade::UpgradeManager::current_wasm_hash(env)
    }

    pub fn current_version(env: Env) -> u32 {
        upgrade::UpgradeManager::current_version(env)
    }
}
