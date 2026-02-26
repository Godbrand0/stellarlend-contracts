//! # Multisig Module
//!
//! Implements a proposal → approve → execute governance flow
//! for updating critical StellarLend protocol parameters.
//!
//! ## How It Works
//! Sensitive changes (for example, updating the minimum collateral ratio)
//! must go through a multisig approval process:
//!
//! 1. Configure the admin set and approval threshold via [`ms_set_admins`].
//! 2. An admin creates a proposal with [`ms_propose_set_min_cr`]
//!    (the proposer auto-approves).
//! 3. Other admins approve using [`ms_approve`] until the threshold is met.
//! 4. Any admin executes the proposal with [`ms_execute`].
//!
//! ## Safety Guarantees
//! - Only registered admins can propose, approve, or update the admin set.
//! - Each admin can approve a proposal only once.
//! - Proposal IDs are strictly increasing and never reused.
//! - Executed proposals cannot be run again.
//! - Only one active proposal can exist at a time.

#![allow(unused)]
use soroban_sdk::{Address, Env, Symbol, Vec};

use crate::governance::{
    approve_proposal, create_proposal, emit_approval_event, emit_proposal_executed_event,
    execute_multisig_proposal, execute_proposal, get_multisig_admins, get_multisig_threshold,
    get_proposal, get_proposal_approvals, set_multisig_admins, set_multisig_threshold,
    GovernanceDataKey, GovernanceError, Proposal, ProposalStatus, ProposalType,
};

// ============================================================================
// Admin Management
// ============================================================================

/// Replaces the multisig admin list and approval threshold.
///
/// During initial setup (when no admins exist yet), any caller may
/// initialize the admin set. After that, only an existing admin
/// can modify it.
///
/// The admin list must be non-empty, contain no duplicates,
/// and the threshold must be between 1 and the number of admins.
///
/// # Errors
/// - [`GovernanceError::Unauthorized`] if a non-admin tries to modify
///   the set after bootstrap.
/// - [`GovernanceError::InvalidMultisigConfig`] if the list is empty,
///   contains duplicates, or the threshold is invalid.
pub fn ms_set_admins(
    env: &Env,
    caller: Address,
    admins: Vec<Address>,
    threshold: u32,
) -> Result<(), GovernanceError> {
    if admins.is_empty() {
        return Err(GovernanceError::InvalidMultisigConfig);
    }
    if threshold == 0 || threshold > admins.len() {
        return Err(GovernanceError::InvalidMultisigConfig);
    }

    // Duplicate check
    for i in 0..admins.len() {
        for j in (i + 1)..admins.len() {
            if admins.get(i).unwrap() == admins.get(j).unwrap() {
                return Err(GovernanceError::InvalidMultisigConfig);
            }
        }
    }

    let existing = get_multisig_admins(env);
    if existing.is_none() {
        // Bootstrap — accept any caller, just persist the list
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::MultisigAdmins, &admins);
    } else {
        // Post-bootstrap — must be an existing admin
        set_multisig_admins(env, caller.clone(), admins.clone())?;
    }

    // Persist threshold (bypassing the set_multisig_threshold guard so we
    // can set it during bootstrap before the admin list is validated)
    env.storage()
        .persistent()
        .set(&GovernanceDataKey::MultisigThreshold, &threshold);

    Ok(())
}

/// Creates a proposal to update the minimum collateral ratio.
///
/// The proposer must be a registered admin. Once created,
/// the proposal is automatically approved by the proposer.
/// Only one active proposal can exist at a time.
///
/// `new_ratio` is expressed in basis points
/// (e.g. 15000 = 150%) and must be greater than 100%.

/// # Returns
/// The ID of the newly created proposal.
///
/// # Errors
/// - [`GovernanceError::Unauthorized`] if the caller is not an admin.
/// - [`GovernanceError::InvalidProposal`] if the ratio is economically invalid
///   or proposal creation fails.

pub fn ms_propose_set_min_cr(
    env: &Env,
    proposer: Address,
    new_ratio: i128,
) -> Result<u64, GovernanceError> {
    if new_ratio <= 10_000 {
        return Err(GovernanceError::InvalidProposal);
    }

    // Delegates auth check + proposal creation to governance.rs
    let proposal_id =
        crate::governance::propose_set_min_collateral_ratio(env, proposer.clone(), new_ratio)?;

    // Proposer auto-approves their own proposal
    approve_proposal(env, proposer, proposal_id)?;

    Ok(proposal_id)
}

// ============================================================================
// Approve
// ============================================================================

/// Approves an existing multisig proposal.
///
/// Each admin may approve a given proposal only once.
/// Validation and event emission are handled internally.
///
/// # Errors
/// - [`GovernanceError::Unauthorized`] if the caller is not an admin.
/// - [`GovernanceError::ProposalNotFound`] if the proposal does not exist.
/// - [`GovernanceError::AlreadyVoted`] if the admin already approved.
pub fn ms_approve(env: &Env, approver: Address, proposal_id: u64) -> Result<(), GovernanceError> {
    approve_proposal(env, approver, proposal_id)
}

// ============================================================================
// Execute
// ============================================================================

/// Executes a multisig proposal once it has enough approvals.
///
/// The caller must be a registered admin. This function verifies
/// that the approval threshold has been met, then applies the
/// proposal’s changes and marks it as executed.
///
/// # Errors
/// - [`GovernanceError::Unauthorized`] if the caller is not an admin.
/// - [`GovernanceError::InsufficientApprovals`] if the threshold
///   has not been reached.
/// - [`GovernanceError::ProposalAlreadyExecuted`] if the proposal
///   was already executed.
/// - [`GovernanceError::ProposalNotReady`] if a timelock is still active.
pub fn ms_execute(env: &Env, executor: Address, proposal_id: u64) -> Result<(), GovernanceError> {
    execute_multisig_proposal(env, executor, proposal_id)
}

// ============================================================================
// View Functions
// ============================================================================

/// Returns the current multisig admin list, if initialized.
pub fn get_ms_admins(env: &Env) -> Option<Vec<Address>> {
    get_multisig_admins(env)
}

/// Return the multisig approval threshold (defaults to `1`).
pub fn get_ms_threshold(env: &Env) -> u32 {
    get_multisig_threshold(env)
}

/// Returns a proposal by its ID, if it exists.
pub fn get_ms_proposal(env: &Env, proposal_id: u64) -> Option<Proposal> {
    get_proposal(env, proposal_id)
}

/// Return the list of admins who have approved a proposal, or `None` if not found.
pub fn get_ms_approvals(env: &Env, proposal_id: u64) -> Option<Vec<Address>> {
    get_proposal_approvals(env, proposal_id)
}
