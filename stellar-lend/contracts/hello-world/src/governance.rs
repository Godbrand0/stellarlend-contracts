#![allow(unused_variables)]

use soroban_sdk::{token::TokenClient, Address, Env, String, Symbol, Val, Vec};

use crate::errors::GovernanceError;
use crate::storage::{GovernanceDataKey, GuardianConfig};

use crate::events::{
    GovernanceInitializedEvent, GuardianAddedEvent, GuardianRemovedEvent, ProposalApprovedEvent,
    ProposalCancelledEvent, ProposalCreatedEvent, ProposalExecutedEvent, ProposalFailedEvent,
    ProposalQueuedEvent, RecoveryApprovedEvent, RecoveryExecutedEvent, RecoveryStartedEvent,
    VoteCastEvent,
};

use crate::types::{
    Action, GovernanceConfig, MultisigConfig, Proposal, ProposalOutcome, ProposalStatus,
    ProposalType, RecoveryRequest, Vote, VoteInfo, VoteType, BASIS_POINTS_SCALE,
    DEFAULT_EXECUTION_DELAY, DEFAULT_QUORUM_BPS, DEFAULT_RECOVERY_PERIOD,
    DEFAULT_TIMELOCK_DURATION, DEFAULT_VOTING_PERIOD, DEFAULT_VOTING_THRESHOLD,
};

// ========================================================================
// Initialization
// ========================================================================

pub fn initialize(
    env: &Env,
    admin: Address,
    vote_token: Address,
    voting_period: Option<u64>,
    execution_delay: Option<u64>,
    quorum_bps: Option<u32>,
    proposal_threshold: Option<i128>,
    timelock_duration: Option<u64>,
    default_voting_threshold: Option<i128>,
) -> Result<(), GovernanceError> {
    if env.storage().instance().has(&GovernanceDataKey::Admin) {
        return Err(GovernanceError::AlreadyInitialized);
    }

    admin.require_auth();

    let config = GovernanceConfig {
        voting_period: voting_period.unwrap_or(DEFAULT_VOTING_PERIOD),
        execution_delay: execution_delay.unwrap_or(DEFAULT_EXECUTION_DELAY),
        quorum_bps: quorum_bps.unwrap_or(DEFAULT_QUORUM_BPS),
        proposal_threshold: proposal_threshold.unwrap_or(0),
        vote_token,
        timelock_duration: timelock_duration.unwrap_or(DEFAULT_TIMELOCK_DURATION),
        default_voting_threshold: default_voting_threshold.unwrap_or(DEFAULT_VOTING_THRESHOLD),
    };

    if config.quorum_bps > 10000 {
        return Err(GovernanceError::InvalidQuorum);
    }
    if config.voting_period == 0 {
        return Err(GovernanceError::InvalidVotingPeriod);
    }

    env.storage()
        .instance()
        .set(&GovernanceDataKey::Admin, &admin);
    env.storage()
        .instance()
        .set(&GovernanceDataKey::Config, &config);
    env.storage()
        .instance()
        .set(&GovernanceDataKey::NextProposalId, &0u64);

    let mut admins = Vec::new(env);
    admins.push_back(admin.clone());
    let multisig_config = MultisigConfig {
        admins,
        threshold: 1,
    };
    env.storage()
        .instance()
        .set(&GovernanceDataKey::MultisigConfig, &multisig_config);

    let guardian_config = GuardianConfig {
        guardians: Vec::new(env),
        threshold: 1,
    };
    env.storage()
        .instance()
        .set(&GovernanceDataKey::GuardianConfig, &guardian_config);

    GovernanceInitializedEvent {
        admin,
        vote_token: config.vote_token,
        voting_period: config.voting_period,
        quorum_bps: config.quorum_bps,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

// ========================================================================
// Proposal Creation
// ========================================================================

pub fn create_proposal(
    env: &Env,
    proposer: Address,
    proposal_type: ProposalType,
    description: String,
    voting_threshold: Option<i128>,
) -> Result<u64, GovernanceError> {
    proposer.require_auth();

    let config: GovernanceConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Config)
        .ok_or(GovernanceError::NotInitialized)?;

    if config.proposal_threshold > 0 {
        let token_client = TokenClient::new(env, &config.vote_token);
        let balance = token_client.balance(&proposer);

        if balance < config.proposal_threshold {
            return Err(GovernanceError::InsufficientProposalPower);
        }
    }

    let next_id: u64 = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::NextProposalId)
        .unwrap_or(0);

    let now = env.ledger().timestamp();

    let proposal = Proposal {
        id: next_id,
        proposer: proposer.clone(),
        proposal_type,
        description: description.clone(),
        status: ProposalStatus::Pending,
        start_time: now,
        end_time: now + config.voting_period,
        execution_time: None,
        voting_threshold: voting_threshold.unwrap_or(config.default_voting_threshold),
        for_votes: 0,
        against_votes: 0,
        abstain_votes: 0,
        total_voting_power: 0,
        created_at: now,
    };

    env.storage()
        .persistent()
        .set(&GovernanceDataKey::Proposal(next_id), &proposal);

    let user_key = GovernanceDataKey::UserProposals(proposer.clone(), next_id);
    env.storage().persistent().set(&user_key, &true);

    let approvals_key = GovernanceDataKey::ProposalApprovals(next_id);
    let approvals: Vec<Address> = Vec::new(env);
    env.storage().persistent().set(&approvals_key, &approvals);

    env.storage()
        .instance()
        .set(&GovernanceDataKey::NextProposalId, &(next_id + 1));

    ProposalCreatedEvent {
        proposal_id: next_id,
        proposer,
        proposal_type: proposal.proposal_type,
        description,
        start_time: proposal.start_time,
        end_time: proposal.end_time,
        created_at: now,
    }
    .publish(env);

    Ok(next_id)
}

// ========================================================================
// Voting
// ========================================================================

pub fn vote(
    env: &Env,
    voter: Address,
    proposal_id: u64,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    let config: GovernanceConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Config)
        .ok_or(GovernanceError::NotInitialized)?;

    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&GovernanceDataKey::Proposal(proposal_id))
        .ok_or(GovernanceError::ProposalNotFound)?;

    let now = env.ledger().timestamp();

    if proposal.status == ProposalStatus::Pending && now >= proposal.start_time {
        proposal.status = ProposalStatus::Active;
    }

    if proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }

    let vote_key = GovernanceDataKey::Vote(proposal_id, voter.clone());
    if env.storage().persistent().has(&vote_key) {
        return Err(GovernanceError::AlreadyVoted);
    }

    let token_client = TokenClient::new(env, &config.vote_token);
    let voting_power = token_client.balance(&voter);

    if voting_power == 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    match vote_type {
        VoteType::For => proposal.for_votes += voting_power,
        VoteType::Against => proposal.against_votes += voting_power,
        VoteType::Abstain => proposal.abstain_votes += voting_power,
    }
    proposal.total_voting_power += voting_power;

    env.storage()
        .persistent()
        .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
    env.storage().persistent().set(
        &vote_key,
        &VoteInfo {
            voter: voter.clone(),
            proposal_id,
            vote_type: vote_type.clone(),
            voting_power,
            timestamp: now,
        },
    );

    VoteCastEvent {
        proposal_id,
        voter,
        vote_type,
        voting_power,
        timestamp: now,
    }
    .publish(env);

    Ok(())
}

// ========================================================================
// Queue Proposal
// ========================================================================

pub fn queue_proposal(
    env: &Env,
    caller: Address,
    proposal_id: u64,
) -> Result<ProposalOutcome, GovernanceError> {
    caller.require_auth();

    let config: GovernanceConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Config)
        .ok_or(GovernanceError::NotInitialized)?;

    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&GovernanceDataKey::Proposal(proposal_id))
        .ok_or(GovernanceError::ProposalNotFound)?;

    let now = env.ledger().timestamp();

    if now <= proposal.end_time {
        return Err(GovernanceError::VotingNotEnded);
    }

    match proposal.status {
        ProposalStatus::Executed
        | ProposalStatus::Cancelled
        | ProposalStatus::Expired
        | ProposalStatus::Queued => {
            return Err(GovernanceError::InvalidProposalStatus);
        }
        _ => {}
    }

    if now > proposal.end_time + DEFAULT_TIMELOCK_DURATION {
        proposal.status = ProposalStatus::Expired;
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
        return Err(GovernanceError::ProposalExpired);
    }

    let total_votes = proposal.for_votes + proposal.against_votes + proposal.abstain_votes;
    let quorum_required = (total_votes * config.quorum_bps as i128) / BASIS_POINTS_SCALE;
    let quorum_reached = total_votes >= quorum_required;

    let threshold_votes =
        (proposal.total_voting_power * proposal.voting_threshold) / BASIS_POINTS_SCALE;
    let threshold_met = proposal.for_votes >= threshold_votes;

    let succeeded = quorum_reached && threshold_met;

    let outcome = ProposalOutcome {
        proposal_id,
        succeeded,
        for_votes: proposal.for_votes,
        against_votes: proposal.against_votes,
        abstain_votes: proposal.abstain_votes,
        quorum_reached,
        quorum_required,
    };

    if succeeded {
        let execution_time = now + config.execution_delay;
        proposal.execution_time = Some(execution_time);
        proposal.status = ProposalStatus::Queued;

        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

        ProposalQueuedEvent {
            proposal_id,
            execution_time,
            for_votes: proposal.for_votes,
            against_votes: proposal.against_votes,
            quorum_reached: outcome.quorum_reached,
            threshold_met: outcome.succeeded && outcome.quorum_reached,
        }
        .publish(env);
    } else {
        proposal.status = ProposalStatus::Defeated;
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

        ProposalFailedEvent {
            proposal_id,
            for_votes: proposal.for_votes,
            against_votes: proposal.against_votes,
            quorum_reached,
            threshold_met: !succeeded && quorum_reached,
        }
        .publish(env);
    }

    Ok(outcome)
}

// ========================================================================
// Execute Proposal
// ========================================================================

pub fn execute_proposal(
    env: &Env,
    executor: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    executor.require_auth();

    let config: GovernanceConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Config)
        .ok_or(GovernanceError::NotInitialized)?;

    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&GovernanceDataKey::Proposal(proposal_id))
        .ok_or(GovernanceError::ProposalNotFound)?;

    let now = env.ledger().timestamp();

    if proposal.status != ProposalStatus::Queued {
        return Err(GovernanceError::NotQueued);
    }

    let execution_time = proposal
        .execution_time
        .ok_or(GovernanceError::InvalidExecutionTime)?;

    if now < execution_time {
        return Err(GovernanceError::ExecutionTooEarly);
    }

    if now > execution_time + config.timelock_duration {
        proposal.status = ProposalStatus::Expired;
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);
        return Err(GovernanceError::ProposalExpired);
    }

    execute_proposal_type(env, &proposal.proposal_type)?;

    proposal.status = ProposalStatus::Executed;
    env.storage()
        .persistent()
        .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

    ProposalExecutedEvent {
        proposal_id,
        executor,
        timestamp: now,
    }
    .publish(env);

    Ok(())
}

fn execute_proposal_type(env: &Env, proposal_type: &ProposalType) -> Result<(), GovernanceError> {
    match proposal_type {
        ProposalType::MinCollateralRatio(val) => {
            crate::risk_params::set_risk_params(env, Some(*val), None, None, None)
                .map_err(|_| GovernanceError::ExecutionFailed)?;
        }
        ProposalType::RiskParams(min_cr, liq_threshold, close_factor, liq_incentive) => {
            crate::risk_params::set_risk_params(
                env,
                *min_cr,
                *liq_threshold,
                *close_factor,
                *liq_incentive,
            )
            .map_err(|_| GovernanceError::ExecutionFailed)?;
        }
        ProposalType::AssetConfigUpdate(asset, cf, lt, ms, mb, cc, cb) => {
            crate::cross_asset::update_asset_config(
                env,
                asset.clone(),
                *cf,
                *lt,
                *ms,
                *mb,
                *cc,
                *cb,
            )
            .map_err(|_| GovernanceError::ExecutionFailed)?;
        }
        ProposalType::PauseSwitch(op, paused) => {
            let admin = env.current_contract_address();
            crate::risk_management::set_pause_switch(env, admin, op.clone(), *paused)
                .map_err(|_| GovernanceError::ExecutionFailed)?;
        }
        ProposalType::EmergencyPause(paused) => {
            let admin = env.current_contract_address();
            crate::risk_management::set_emergency_pause(env, admin, *paused)
                .map_err(|_| GovernanceError::ExecutionFailed)?;
        }
        ProposalType::GenericAction(action) => {
            execute_generic_action(env, action)?;
        }
    }
    Ok(())
}

fn execute_generic_action(env: &Env, action: &Action) -> Result<(), GovernanceError> {
    env.invoke_contract::<Val>(&action.target, &action.method, action.args.clone());
    Ok(())
}

// ========================================================================
// Cancel Proposal
// ========================================================================

pub fn cancel_proposal(
    env: &Env,
    caller: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let admin: Address = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    let mut proposal: Proposal = env
        .storage()
        .persistent()
        .get(&GovernanceDataKey::Proposal(proposal_id))
        .ok_or(GovernanceError::ProposalNotFound)?;

    if caller != proposal.proposer && caller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    match proposal.status {
        ProposalStatus::Executed | ProposalStatus::Queued => {
            return Err(GovernanceError::InvalidProposalStatus);
        }
        _ => {}
    }

    proposal.status = ProposalStatus::Cancelled;
    env.storage()
        .persistent()
        .set(&GovernanceDataKey::Proposal(proposal_id), &proposal);

    ProposalCancelledEvent {
        proposal_id,
        caller,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

// ========================================================================
// Multisig Operations
// ========================================================================

pub fn approve_proposal(
    env: &Env,
    approver: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    approver.require_auth();

    let multisig_config: MultisigConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::MultisigConfig)
        .ok_or(GovernanceError::NotInitialized)?;

    if !multisig_config.admins.contains(&approver) {
        return Err(GovernanceError::Unauthorized);
    }

    let proposal_key = GovernanceDataKey::Proposal(proposal_id);
    if !env.storage().persistent().has(&proposal_key) {
        return Err(GovernanceError::ProposalNotFound);
    }

    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    let mut approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or_else(|| Vec::new(env));

    if approvals.contains(&approver) {
        return Err(GovernanceError::AlreadyVoted);
    }

    approvals.push_back(approver.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    ProposalApprovedEvent {
        proposal_id,
        approver,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

pub fn set_multisig_config(
    env: &Env,
    caller: Address,
    admins: Vec<Address>,
    threshold: u32,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let admin: Address = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    if caller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    if admins.is_empty() || threshold == 0 || threshold > admins.len() {
        return Err(GovernanceError::InvalidMultisigConfig);
    }

    let config = MultisigConfig { admins, threshold };
    env.storage()
        .instance()
        .set(&GovernanceDataKey::MultisigConfig, &config);

    Ok(())
}

/// Return the list of admins who have approved a proposal, or `None` if not found.
pub fn get_proposal_approvals(env: &Env, proposal_id: u64) -> Option<Vec<Address>> {
    let approvals_key = GovernanceDataKey::ProposalApprovals(proposal_id);
    env.storage().persistent().get(&approvals_key)
}

// ============================================================================
// Events
// ============================================================================

fn emit_proposal_created_event(env: &Env, proposal_id: &u64, proposer: &Address) {
    let topics = (
        Symbol::new(env, "proposal_created"),
        *proposal_id,
        proposer.clone(),
    );
    env.events().publish(topics, ());
}

fn emit_vote_cast_event(
    env: &Env,
    proposal_id: &u64,
    voter: &Address,
    vote: &Vote,
    voting_power: &i128,
) {
    let topics = (Symbol::new(env, "vote_cast"), *proposal_id, voter.clone());
    env.events().publish(topics, (vote.clone(), *voting_power));
}

pub fn emit_proposal_executed_event(env: &Env, proposal_id: &u64, executor: &Address) {
    let topics = (
        Symbol::new(env, "proposal_executed"),
        *proposal_id,
        executor.clone(),
    );
    env.events().publish(topics, ());
}

fn emit_proposal_failed_event(env: &Env, proposal_id: &u64) {
    let topics = (Symbol::new(env, "proposal_failed"), *proposal_id);
    env.events().publish(topics, ());
}

pub fn emit_approval_event(env: &Env, proposal_id: &u64, approver: &Address) {
    let topics = (
        Symbol::new(env, "proposal_approved"),
        *proposal_id,
        approver.clone(),
    );
    env.events().publish(topics, ());
}

pub fn add_guardian(env: &Env, caller: Address, guardian: Address) -> Result<(), GovernanceError> {
    caller.require_auth();

    let admin: Address = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    if caller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    let mut guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .unwrap_or_else(|| GuardianConfig {
            guardians: Vec::new(env),
            threshold: 1,
        });

    if guardian_config.guardians.contains(&guardian) {
        return Err(GovernanceError::GuardianAlreadyExists);
    }

    guardian_config.guardians.push_back(guardian.clone());
    env.storage()
        .instance()
        .set(&GovernanceDataKey::GuardianConfig, &guardian_config);

    GuardianAddedEvent {
        guardian,
        added_by: caller,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

pub fn remove_guardian(
    env: &Env,
    caller: Address,
    guardian: Address,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let admin: Address = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    if caller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    let mut guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .ok_or(GovernanceError::GuardianNotFound)?;

    let mut new_guardians = Vec::new(env);
    let mut found = false;

    for g in guardian_config.guardians.iter() {
        if g != guardian {
            new_guardians.push_back(g);
        } else {
            found = true;
        }
    }

    if !found {
        return Err(GovernanceError::GuardianNotFound);
    }

    guardian_config.guardians = new_guardians;

    if guardian_config.threshold > guardian_config.guardians.len() {
        guardian_config.threshold = guardian_config.guardians.len();
    }

    env.storage()
        .instance()
        .set(&GovernanceDataKey::GuardianConfig, &guardian_config);

    GuardianRemovedEvent {
        guardian,
        removed_by: caller,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

pub fn set_guardian_threshold(
    env: &Env,
    caller: Address,
    threshold: u32,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let admin: Address = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    if caller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    let mut guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .ok_or(GovernanceError::GuardianNotFound)?;

    if threshold == 0 || threshold > guardian_config.guardians.len() {
        return Err(GovernanceError::InvalidGuardianConfig);
    }

    guardian_config.threshold = threshold;
    env.storage()
        .instance()
        .set(&GovernanceDataKey::GuardianConfig, &guardian_config);

    Ok(())
}

pub fn start_recovery(
    env: &Env,
    initiator: Address,
    old_admin: Address,
    new_admin: Address,
) -> Result<(), GovernanceError> {
    initiator.require_auth();

    let guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .ok_or(GovernanceError::GuardianNotFound)?;

    if !guardian_config.guardians.contains(&initiator) {
        return Err(GovernanceError::Unauthorized);
    }

    let recovery_key = GovernanceDataKey::RecoveryRequest;
    if env.storage().persistent().has(&recovery_key) {
        return Err(GovernanceError::RecoveryInProgress);
    }

    let now = env.ledger().timestamp();
    let request = RecoveryRequest {
        old_admin,
        new_admin: new_admin.clone(),
        initiator: initiator.clone(),
        initiated_at: now,
        expires_at: now + DEFAULT_RECOVERY_PERIOD,
    };

    env.storage().persistent().set(&recovery_key, &request);

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let mut approvals = Vec::new(env);
    approvals.push_back(initiator.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    RecoveryStartedEvent {
        old_admin: request.old_admin,
        new_admin,
        initiator,
        expires_at: request.expires_at,
        timestamp: now,
    }
    .publish(env);

    Ok(())
}

pub fn approve_recovery(env: &Env, approver: Address) -> Result<(), GovernanceError> {
    approver.require_auth();

    let guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .ok_or(GovernanceError::GuardianNotFound)?;

    if !guardian_config.guardians.contains(&approver) {
        return Err(GovernanceError::Unauthorized);
    }

    let recovery_key = GovernanceDataKey::RecoveryRequest;
    if !env.storage().persistent().has(&recovery_key) {
        return Err(GovernanceError::NoRecoveryInProgress);
    }

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let mut approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or_else(|| Vec::new(env));

    if approvals.contains(&approver) {
        return Err(GovernanceError::AlreadyVoted);
    }

    approvals.push_back(approver.clone());
    env.storage().persistent().set(&approvals_key, &approvals);

    RecoveryApprovedEvent {
        approver,
        current_approvals: approvals.len(),
        threshold: guardian_config.threshold,
        timestamp: env.ledger().timestamp(),
    }
    .publish(env);

    Ok(())
}

pub fn execute_recovery(env: &Env, executor: Address) -> Result<(), GovernanceError> {
    executor.require_auth();

    let guardian_config: GuardianConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::GuardianConfig)
        .ok_or(GovernanceError::GuardianNotFound)?;

    let recovery_key = GovernanceDataKey::RecoveryRequest;
    let request: RecoveryRequest = env
        .storage()
        .persistent()
        .get(&recovery_key)
        .ok_or(GovernanceError::NoRecoveryInProgress)?;

    let now = env.ledger().timestamp();
    if now > request.expires_at {
        env.storage().persistent().remove(&recovery_key);
        return Err(GovernanceError::ProposalExpired);
    }

    let approvals_key = GovernanceDataKey::RecoveryApprovals;
    let approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&approvals_key)
        .unwrap_or_else(|| Vec::new(env));

    if approvals.len() < guardian_config.threshold {
        return Err(GovernanceError::InsufficientApprovals);
    }

    let mut multisig_config: MultisigConfig = env
        .storage()
        .instance()
        .get(&GovernanceDataKey::MultisigConfig)
        .ok_or(GovernanceError::NotInitialized)?;

    let mut new_admins = Vec::new(env);
    for admin in multisig_config.admins.iter() {
        if admin != request.old_admin {
            new_admins.push_back(admin);
        }
    }
    new_admins.push_back(request.new_admin.clone());

    multisig_config.admins = new_admins;
    env.storage()
        .instance()
        .set(&GovernanceDataKey::MultisigConfig, &multisig_config);

    env.storage().persistent().remove(&recovery_key);
    env.storage().persistent().remove(&approvals_key);

    RecoveryExecutedEvent {
        old_admin: request.old_admin,
        new_admin: request.new_admin,
        executor,
        timestamp: now,
    }
    .publish(env);

    Ok(())
}

// ========================================================================
// Query Functions
// ========================================================================

pub fn get_proposal(env: &Env, proposal_id: u64) -> Option<Proposal> {
    env.storage()
        .persistent()
        .get(&GovernanceDataKey::Proposal(proposal_id))
}

pub fn get_vote(env: &Env, proposal_id: u64, voter: Address) -> Option<VoteInfo> {
    env.storage()
        .persistent()
        .get(&GovernanceDataKey::Vote(proposal_id, voter))
}

pub fn get_config(env: &Env) -> Option<GovernanceConfig> {
    env.storage().instance().get(&GovernanceDataKey::Config)
}

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&GovernanceDataKey::Admin)
}

pub fn get_multisig_config(env: &Env) -> Option<MultisigConfig> {
    env.storage()
        .instance()
        .get(&GovernanceDataKey::MultisigConfig)
}

pub fn emit_guardian_added_event(env: &Env, guardian: &Address) {
    let topics = (Symbol::new(env, "guardian_added"), guardian.clone());
    env.events().publish(topics, ());
}

pub fn emit_guardian_removed_event(env: &Env, guardian: &Address) {
    let topics = (Symbol::new(env, "guardian_removed"), guardian.clone());
    env.events().publish(topics, ());
}

pub fn emit_recovery_started_event(
    env: &Env,
    old_admin: &Address,
    new_admin: &Address,
    initiator: &Address,
) {
    let topics = (
        Symbol::new(env, "recovery_started"),
        old_admin.clone(),
        new_admin.clone(),
    );
    env.events().publish(topics, initiator.clone());
}

pub fn emit_recovery_approved_event(env: &Env, approver: &Address) {
    let topics = (Symbol::new(env, "recovery_approved"), approver.clone());
    env.events().publish(topics, ());
}

pub fn emit_recovery_executed_event(
    env: &Env,
    old_admin: &Address,
    new_admin: &Address,
    executor: &Address,
) {
    let topics = (
        Symbol::new(env, "recovery_executed"),
        old_admin,
        new_admin,
        executor,
    );
    env.events().publish(topics, ());
}

// Wrapper functions for multisig operations to maintain compatibility
pub fn get_multisig_admins(env: &Env) -> Option<Vec<Address>> {
    crate::multisig::get_ms_admins(env)
}

pub fn get_multisig_threshold(env: &Env) -> u32 {
    crate::multisig::get_ms_threshold(env)
}

pub fn get_guardian_config(env: &Env) -> Option<GuardianConfig> {
    crate::storage::get_guardian_config(env)
}

pub fn get_recovery_request(env: &Env) -> Option<RecoveryRequest> {
    crate::storage::get_recovery_request(env)
}

pub fn get_recovery_approvals(env: &Env) -> Option<Vec<Address>> {
    crate::storage::get_recovery_approvals(env)
}

pub fn get_proposals(env: &Env, start_id: u64, limit: u32) -> Vec<Proposal> {
    crate::storage::get_proposals(env, start_id, limit)
}

pub fn can_vote(env: &Env, voter: Address, proposal_id: u64) -> bool {
    crate::storage::can_vote(env, voter, proposal_id)
}

pub fn set_multisig_admins(
    env: &Env,
    caller: Address,
    admins: Vec<Address>,
    threshold: u32,
) -> Result<(), GovernanceError> {
    crate::multisig::ms_set_admins(env, caller, admins, threshold)
}

pub fn set_multisig_threshold(
    env: &Env,
    caller: Address,
    threshold: u32,
) -> Result<(), GovernanceError> {
    crate::multisig::set_ms_threshold(env, caller, threshold)
}

pub fn execute_multisig_proposal(
    env: &Env,
    executor: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    crate::multisig::ms_execute(env, executor, proposal_id)
}

pub fn propose_set_min_collateral_ratio(
    env: &Env,
    proposer: Address,
    new_ratio: u32,
) -> Result<u64, GovernanceError> {
    crate::multisig::ms_propose_set_min_cr(env, proposer, new_ratio.into())
}
