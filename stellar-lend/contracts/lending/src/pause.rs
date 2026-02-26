use soroban_sdk::{contractevent, contracttype, Address, Env};

/// Types of operations that can be paused.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PauseType {
    /// Pause all protocol operations
    All = 0,
    /// Pause deposit operations
    Deposit = 1,
    /// Pause borrow operations
    Borrow = 2,
    /// Pause repay operations
    Repay = 3,
    /// Pause withdraw operations
    Withdraw = 4,
    /// Pause liquidation operations
    Liquidation = 5,
}

/// Emergency lifecycle states for protocol-wide incident handling.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EmergencyState {
    /// Protocol operates normally.
    Normal = 0,
    /// High-risk operations are hard-stopped.
    Shutdown = 1,
    /// Controlled unwind mode for user recovery.
    Recovery = 2,
}

/// Storage keys for pause states.
#[contracttype]
#[derive(Clone)]
pub enum PauseDataKey {
    /// Pause state for a specific operation type
    State(PauseType),
    /// Optional guardian address authorized to trigger emergency shutdown.
    Guardian,
    /// Current emergency lifecycle state.
    EmergencyState,
}

/// Event data emitted on pause state change.
#[contractevent]
#[derive(Clone, Debug)]
pub struct PauseEvent {
    /// Operation type affected
    pub pause_type: PauseType,
    /// New pause state
    pub paused: bool,
    /// Admin who performed the action
    pub admin: Address,
}

/// Event emitted whenever guardian configuration changes.
#[contractevent]
#[derive(Clone, Debug)]
pub struct GuardianSetEvent {
    /// Guardian address newly configured by admin.
    pub guardian: Address,
    /// Admin who set the guardian.
    pub admin: Address,
}

/// Event emitted on emergency state transitions.
#[contractevent]
#[derive(Clone, Debug)]
pub struct EmergencyStateEvent {
    /// Previous emergency state.
    pub from: EmergencyState,
    /// New emergency state.
    pub to: EmergencyState,
    /// Caller that triggered transition.
    pub caller: Address,
}

/// Set pause state for a specific operation type
///
/// # Arguments
/// * `env` - The contract environment
/// * `admin` - The admin address (must authorize)
/// * `pause_type` - The operation type to pause/unpause
/// * `paused` - True to pause, false to unpause
pub fn set_pause(env: &Env, admin: Address, pause_type: PauseType, paused: bool) {
    // Store the pause state
    env.storage()
        .persistent()
        .set(&PauseDataKey::State(pause_type), &paused);

    // Emit event
    PauseEvent {
        pause_type,
        paused,
        admin,
    }
    .publish(env);
}

/// Check if a specific operation is paused
///
/// An operation is considered paused if either its specific pause flag
/// is set or the global `All` pause flag is set.
///
/// # Arguments
/// * `env` - The contract environment
/// * `pause_type` - The operation type to check
///
/// # Returns
/// True if paused, false otherwise
pub fn is_paused(env: &Env, pause_type: PauseType) -> bool {
    // Check global pause first
    if env
        .storage()
        .persistent()
        .get(&PauseDataKey::State(PauseType::All))
        .unwrap_or(false)
    {
        return true;
    }

    // Check specific operation pause
    if pause_type != PauseType::All {
        return env
            .storage()
            .persistent()
            .get(&PauseDataKey::State(pause_type))
            .unwrap_or(false);
    }

    false
}

/// Set or rotate guardian authorized to trigger emergency shutdown.
pub fn set_guardian(env: &Env, admin: Address, guardian: Address) {
    env.storage()
        .persistent()
        .set(&PauseDataKey::Guardian, &guardian);
    GuardianSetEvent { guardian, admin }.publish(env);
}

/// Return currently configured guardian, if any.
pub fn get_guardian(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&PauseDataKey::Guardian)
}

/// Return current emergency state. Defaults to normal operation.
pub fn get_emergency_state(env: &Env) -> EmergencyState {
    env.storage()
        .persistent()
        .get(&PauseDataKey::EmergencyState)
        .unwrap_or(EmergencyState::Normal)
}

/// Return true if protocol is in controlled recovery mode.
pub fn is_recovery(env: &Env) -> bool {
    get_emergency_state(env) == EmergencyState::Recovery
}

/// Return true if high-risk operations should be blocked.
pub fn blocks_high_risk_ops(env: &Env) -> bool {
    matches!(
        get_emergency_state(env),
        EmergencyState::Shutdown | EmergencyState::Recovery
    )
}

/// Transition protocol into emergency shutdown.
pub fn trigger_shutdown(env: &Env, caller: Address) {
    set_emergency_state(env, caller, EmergencyState::Shutdown);
}

/// Transition protocol from shutdown to controlled recovery.
pub fn start_recovery(env: &Env, caller: Address) {
    set_emergency_state(env, caller, EmergencyState::Recovery);
}

/// Complete emergency lifecycle and return protocol to normal operation.
pub fn complete_recovery(env: &Env, caller: Address) {
    set_emergency_state(env, caller, EmergencyState::Normal);
}

fn set_emergency_state(env: &Env, caller: Address, to: EmergencyState) {
    let from = get_emergency_state(env);
    env.storage()
        .persistent()
        .set(&PauseDataKey::EmergencyState, &to);
    EmergencyStateEvent { from, to, caller }.publish(env);
}
