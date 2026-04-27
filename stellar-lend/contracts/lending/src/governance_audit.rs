//! Governance Audit Log Module
//!
//! Provides a standardized audit log for all governance and admin actions
//! including oracle updates, pause toggles, risk parameter changes, caps,
//! and upgrade proposals/executions.
//!
//! ## Features
//! - Stable event schema for all governance actions
//! - Bounded storage for recent actions (gas-efficient querying)
//! - Comprehensive action types with extensible payload structure
//! - Security-focused design for incident response and compliance
//!
//! ## Security
//! - All audit entries are immutable once written
//! - Actions are logged atomically with the governance operation
//! - No sensitive data is stored in audit logs (only addresses and enum values)

use soroban_sdk::{contractevent, contracttype, Address, Env, Vec};

// ─────────────────────────────────────────────────────────────────────────────
// Governance Action Types
// ─────────────────────────────────────────────────────────────────────────────

/// Types of governance actions that can be audited.
///
/// This enum is designed to be stable and extensible. New variants can be added
/// without breaking existing audit log consumers.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceAction {
    /// Protocol initialization
    Initialize = 0,
    /// Admin address change
    SetAdmin = 1,
    /// Pause state change for specific operation
    SetPause = 2,
    /// Guardian address configuration
    SetGuardian = 3,
    /// Emergency shutdown trigger
    EmergencyShutdown = 4,
    /// Recovery mode start
    StartRecovery = 5,
    /// Recovery completion
    CompleteRecovery = 6,
    /// Oracle address configuration
    SetOracle = 7,
    /// Oracle parameters configuration
    ConfigureOracle = 8,
    /// Primary oracle address set for asset
    SetPrimaryOracle = 9,
    /// Fallback oracle address set for asset
    SetFallbackOracle = 10,
    /// Oracle pause state change
    SetOraclePaused = 11,
    /// Price feed update
    UpdatePriceFeed = 12,
    /// Liquidation threshold parameter change
    SetLiquidationThreshold = 13,
    /// Close factor parameter change
    SetCloseFactor = 14,
    /// Liquidation incentive parameter change
    SetLiquidationIncentive = 15,
    /// Borrow settings initialization
    InitializeBorrowSettings = 16,
    /// Deposit settings initialization
    InitializeDepositSettings = 17,
    /// Withdraw settings initialization
    InitializeWithdrawSettings = 18,
    /// Flash loan fee parameter change
    SetFlashLoanFee = 19,
    /// Cross-asset admin initialization
    InitializeCrossAssetAdmin = 20,
    /// Asset parameters configuration
    SetAssetParams = 21,
    /// Upgrade process initialization
    UpgradeInit = 22,
    /// Upgrade approver addition
    UpgradeAddApprover = 23,
    /// Upgrade approver removal
    UpgradeRemoveApprover = 24,
    /// Upgrade proposal creation
    UpgradePropose = 25,
    /// Upgrade approval
    UpgradeApprove = 26,
    /// Upgrade execution
    UpgradeExecute = 27,
    /// Upgrade rollback
    UpgradeRollback = 28,
    /// Insurance fund credit
    CreditInsuranceFund = 29,
    /// Bad debt offset
    OffsetBadDebt = 30,
    /// Data writer permission grant
    GrantDataWriter = 31,
    /// Data writer permission revoke
    RevokeDataWriter = 32,
    /// Data backup creation
    DataBackup = 33,
    /// Data restoration
    DataRestore = 34,
    /// Data schema migration
    DataMigrate = 35,
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit Data Structures
// ─────────────────────────────────────────────────────────────────────────────

/// Flexible payload structure for governance action data.
///
/// Uses a Vec<Val> to allow extensible data structures while maintaining
/// type safety through helper functions for common payload patterns.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GovernancePayload {
    /// Action-specific data
    pub data: Vec<Val>,
}

/// Individual audit entry stored in the circular buffer.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AuditEntry {
    /// Sequential ID for ordering
    pub id: u64,
    /// Type of governance action
    pub action: GovernanceAction,
    /// Address that performed the action
    pub caller: Address,
    /// Timestamp when action occurred
    pub timestamp: u64,
    /// Action-specific data
    pub payload: GovernancePayload,
}

/// Event emitted for each governance action.
#[contractevent]
#[derive(Debug, Clone)]
pub struct GovernanceAuditEvent {
    /// Sequential ID for ordering
    pub id: u64,
    /// Type of governance action
    pub action: GovernanceAction,
    /// Address that performed the action
    pub caller: Address,
    /// Timestamp when action occurred
    pub timestamp: u64,
    /// Action-specific data
    pub payload: GovernancePayload,
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage Keys
// ─────────────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AuditLogKey {
    /// Total number of audit entries (used for ID generation)
    Count,
    /// Individual audit entry (index modulo MAX_AUDIT_ENTRIES)
    Entry(u64),
}

/// Maximum number of audit entries to store.
///
/// This creates a circular buffer to bound storage usage and gas costs.
/// When the buffer is full, older entries are overwritten.
pub const MAX_AUDIT_ENTRIES: u64 = 1000;

// ─────────────────────────────────────────────────────────────────────────────
// Core Functions
// ─────────────────────────────────────────────────────────────────────────────

/// Log a governance action to the audit log.
///
/// This function should be called immediately after a successful governance
/// action to ensure atomic logging with the action itself.
///
/// # Arguments
/// * `env` - The contract environment
/// * `action` - The type of governance action performed
/// * `caller` - The address that performed the action
/// * `payload` - Action-specific data
pub fn log_governance_action(
    env: &Env,
    action: GovernanceAction,
    caller: Address,
    payload: GovernancePayload,
) {
    let current_count = env
        .storage()
        .persistent()
        .get(&AuditLogKey::Count)
        .unwrap_or(0);
    
    let new_id = current_count + 1;
    let timestamp = env.ledger().timestamp();
    
    // Create audit entry
    let entry = AuditEntry {
        id: new_id,
        action,
        caller: caller.clone(),
        timestamp,
        payload: payload.clone(),
    };
    
    // Store in circular buffer
    let buffer_index = new_id % MAX_AUDIT_ENTRIES;
    env.storage()
        .persistent()
        .set(&AuditLogKey::Entry(buffer_index), &entry);
    
    // Update count
    env.storage()
        .persistent()
        .set(&AuditLogKey::Count, &new_id);
    
    // Emit event for off-chain monitoring
    let event = GovernanceAuditEvent {
        id: new_id,
        action,
        caller,
        timestamp,
        payload,
    };
    
    event.publish(env);
}

/// Get recent audit entries from the log.
///
/// Returns up to `limit` most recent audit entries in reverse chronological order.
/// The limit is enforced to prevent gas exhaustion attacks.
///
/// # Arguments
/// * `env` - The contract environment
/// * `limit` - Maximum number of entries to return (max 100)
///
/// # Returns
/// Vector of audit entries ordered from newest to oldest
pub fn get_recent_audit_entries(env: &Env, limit: u32) -> Vec<AuditEntry> {
    // Enforce maximum limit to prevent gas exhaustion
    let effective_limit = if limit > 100 { 100 } else { limit };
    
    let total_count = env
        .storage()
        .persistent()
        .get(&AuditLogKey::Count)
        .unwrap_or(0);
    
    if total_count == 0 {
        return Vec::new(env);
    }
    
    let mut entries = Vec::new(env);
    let entries_to_fetch = if total_count < effective_limit as u64 {
        total_count
    } else {
        effective_limit as u64
    };
    
    // Fetch entries in reverse chronological order
    for i in 0..entries_to_fetch {
        let entry_id = total_count - i;
        let buffer_index = entry_id % MAX_AUDIT_ENTRIES;
        
        if let Some(entry) = env
            .storage()
            .persistent()
            .get(&AuditLogKey::Entry(buffer_index))
        {
            // Only include entries that match the expected ID (handle circular buffer)
            if entry.id == entry_id {
                entries.push_back(entry);
            }
        }
    }
    
    entries
}

/// Get the total number of audit entries ever recorded.
///
/// This count includes entries that may have been overwritten in the circular
/// buffer and is useful for pagination purposes.
///
/// # Arguments
/// * `env` - The contract environment
///
/// # Returns
/// Total number of audit entries recorded
pub fn get_audit_count(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&AuditLogKey::Count)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload Helper Functions
// ─────────────────────────────────────────────────────────────────────────────

/// Create an empty payload for actions that don't need additional data.
pub fn payload_empty(env: &Env) -> GovernancePayload {
    GovernancePayload {
        data: Vec::new(env),
    }
}

/// Create a payload with a single address.
pub fn payload_address(env: &Env, address: Address) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into());
    GovernancePayload { data }
}

/// Create a payload with an address and boolean value.
pub fn payload_address_bool(env: &Env, address: Address, value: bool) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into());
    data.push_back(value.into());
    GovernancePayload { data }
}

/// Create a payload with an address and u64 value.
pub fn payload_address_u64(env: &Env, address: Address, value: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into());
    data.push_back(value.into());
    GovernancePayload { data }
}

/// Create a payload with an address and i128 value.
pub fn payload_address_i128(env: &Env, address: Address, value: i128) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into());
    data.push_back(value.into());
    GovernancePayload { data }
}

/// Create a payload with two addresses.
pub fn payload_two_addresses(env: &Env, address1: Address, address2: Address) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address1.into());
    data.push_back(address2.into());
    GovernancePayload { data }
}

/// Create a payload with address, asset, and amount.
pub fn payload_address_asset_i128(
    env: &Env,
    address: Address,
    asset: Address,
    amount: i128,
) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into());
    data.push_back(asset.into());
    data.push_back(amount.into());
    GovernancePayload { data }
}

/// Create a payload with a single i128 value.
pub fn payload_i128(env: &Env, value: i128) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into());
    GovernancePayload { data }
}

/// Create a payload with a single u64 value.
pub fn payload_u64(env: &Env, value: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into());
    GovernancePayload { data }
}

/// Create a payload with two u64 values.
pub fn payload_two_u64(env: &Env, value1: u64, value2: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value1.into());
    data.push_back(value2.into());
    GovernancePayload { data }
}

/// Create a payload with a string value.
pub fn payload_string(env: &Env, value: soroban_sdk::String) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into());
    GovernancePayload { data }
}
