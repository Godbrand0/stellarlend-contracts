//! # Configuration Module
//!
//! Provides key-value configuration storage for the lending protocol.
//! Allows the admin to set, get, backup, and restore configuration parameters.

use crate::risk_management::require_admin;
use soroban_sdk::{contracterror, contracttype, Address, Env, Symbol, Val, Vec};
use crate::prelude::*;

/// Errors that can occur during configuration operations
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ConfigError {
    /// Unauthorized access - caller is not admin
    Unauthorized = 1,
}

/// Storage keys for configuration data
#[contracttype]
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum ConfigDataKey {
    /// Configuration key-value mapping
    ConfigKey(Symbol),
}

/// Set a configuration value (admin only)
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `key` - The configuration key
/// * `value` - The configuration value
///
/// # Returns
/// Returns Ok(()) on success
pub fn config_set(env: &Env, caller: Address, key: Symbol, value: Val) -> Result<(), ConfigError> {
    require_admin(env, &caller).map_err(|_| ConfigError::Unauthorized)?;

    let storage_key = ConfigDataKey::ConfigKey(key);
    env.storage().persistent().set(&storage_key, &value);

    // Consider emitting an event for configuration update, though not explicitly requested, good practice.
    // let topics = (Symbol::new(env, "config_updated"), caller);
    // env.events().publish(topics, value);

    Ok(())
}

/// Get a configuration value
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `key` - The configuration key
///
/// # Returns
/// Returns Some(value) if the key exists, None otherwise
pub fn config_get(env: &Env, key: Symbol) -> Option<Val> {
    let storage_key = ConfigDataKey::ConfigKey(key);
    env.storage().persistent().get(&storage_key)
}

/// Backup configuration parameters (admin only)
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `keys` - A vector of configuration keys to backup
///
/// # Returns
/// Returns a vector of key-value pairs representing the backup
pub fn config_backup(
    env: &Env,
    caller: Address,
    keys: Vec<Symbol>,
) -> Result<Vec<(Symbol, Val)>, ConfigError> {
    require_admin(env, &caller).map_err(|_| ConfigError::Unauthorized)?;

    let mut backup = Vec::new(env);
    for key in keys.iter() {
        if let Some(value) = config_get(env, key.clone()) {
            backup.push_back((key, value));
        }
    }

    Ok(backup)
}

/// Restore configuration parameters (admin only)
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `caller` - The caller address (must be admin)
/// * `backup` - A vector of key-value pairs to restore
///
/// # Returns
/// Returns Ok(()) on success
pub fn config_restore(
    env: &Env,
    caller: Address,
    backup: Vec<(Symbol, Val)>,
) -> Result<(), ConfigError> {
    require_admin(env, &caller).map_err(|_| ConfigError::Unauthorized)?;

    for (key, value) in backup.iter() {
        let storage_key = ConfigDataKey::ConfigKey(key);
        env.storage().persistent().set(&storage_key, &value);
    }

    Ok(())
}
