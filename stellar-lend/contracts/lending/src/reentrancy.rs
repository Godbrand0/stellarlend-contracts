//! # Reentrancy Protection for Lending Contract
//!
//! Soroban executes contract invocations synchronously within a single invocation tree. A
//! token `transfer` or `transfer_from` can therefore call back into this contract before the
//! outer function finishes. This module blocks that shape of nested re-entry by setting a
//! temporary per-contract lock for the duration of the protected frame.
//!
//! The guard does not persist across transactions and does not replace authorization,
//! pause-switch, or collateral checks. It is a defense-in-depth layer for fund-moving entry
//! points that perform external contract calls.

#![allow(unexpected_cfgs)]

use soroban_sdk::{contracttype, Env, Symbol};

/// Standardized error code for reentrancy rejection.
/// This matches the pattern used across the protocol.
pub const REENTRANCY_ERROR_CODE: u32 = 7;

/// Temporary storage key for the reentrancy lock.
///
/// `V1` is explicit so future lock semantics can be introduced without colliding with older
/// layouts. The key lives in temporary storage, so it never participates in persistent state
/// migrations.
#[derive(Clone)]
#[contracttype]
pub enum ReentrancyDataKey {
    LockV1,
}

/// RAII guard that rejects nested entry into protected call paths.
///
/// # Errors
/// Returns [`REENTRANCY_ERROR_CODE`] when the current contract instance is already executing
/// another protected frame in the same invocation tree.
///
/// # Security
/// The lock is scoped to this contract instance and the current transaction only. It blocks
/// synchronous callback re-entry, including re-entry triggered by external token contracts,
/// but it does not create any cross-transaction or cross-contract isolation guarantees.
pub struct ReentrancyGuard<'a> {
    env: &'a Env,
}

impl<'a> ReentrancyGuard<'a> {
    /// Acquires the reentrancy lock for the current protected frame.
    ///
    /// # Errors
    /// Returns [`REENTRANCY_ERROR_CODE`] if the lock is already held by an outer protected
    /// invocation on the same contract instance.
    ///
    /// # Security
    /// Call this before the first external contract call in any function that mutates
    /// protocol-critical state or transfers funds.
    pub fn new(env: &'a Env) -> Result<Self, u32> {
        if is_locked(env) {
            return Err(REENTRANCY_ERROR_CODE);
        }

        env.storage()
            .temporary()
            .set(&ReentrancyDataKey::LockV1, &true);

        Ok(Self { env })
    }

    /// Attempts to acquire the reentrancy guard without panicking.
    ///
    /// Returns `None` if the lock is already held.
    pub fn try_new(env: &'a Env) -> Option<Self> {
        if is_locked(env) {
            return None;
        }

        env.storage()
            .temporary()
            .set(&ReentrancyDataKey::LockV1, &true);

        Some(Self { env })
    }
}

impl Drop for ReentrancyGuard<'_> {
    fn drop(&mut self) {
        self.env
            .storage()
            .temporary()
            .remove(&ReentrancyDataKey::LockV1);
    }
}

impl core::fmt::Debug for ReentrancyGuard<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReentrancyGuard").finish()
    }
}

/// Checks if the reentrancy lock is currently held.
pub fn is_locked(env: &Env) -> bool {
    env.storage().temporary().has(&ReentrancyDataKey::LockV1)
}

/// Helper macro to acquire reentrancy guard at the start of a function.
/// Returns early with error if guard cannot be acquired.
#[macro_export]
macro_rules! require_no_reentrancy {
    ($env:expr, $error:expr) => {{
        if $crate::reentrancy::is_locked($env) {
            return Err($error);
        }
    }};
}

/// Helper macro to acquire reentrancy guard using RAII pattern.
/// Automatically releases the guard when the function exits.
#[macro_export]
macro_rules! reentrancy_guard {
    ($env:expr, $error:expr) => {{
        $crate::reentrancy::ReentrancyGuard::new($env).map_err(|_| $error)?
    }};
}