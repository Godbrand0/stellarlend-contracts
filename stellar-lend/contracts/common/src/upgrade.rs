use soroban_sdk::{
    contracterror, contracttype, panic_with_error, symbol_short, Address, BytesN, Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum UpgradeError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAuthorized = 3,
    ProposalNotFound = 4,
    InvalidVersion = 5,
    InvalidStatus = 6,
    AlreadyApproved = 7,
    NotEnoughApprovals = 8,
    InvalidThreshold = 9,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeStage {
    Proposed,
    Approved,
    Executed,
    RolledBack,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposal {
    pub id: u64,
    pub proposer: Address,
    pub new_wasm_hash: BytesN<32>,
    pub new_version: u32,
    pub approvals: Vec<Address>,
    pub stage: UpgradeStage,
    pub prev_wasm_hash: Option<BytesN<32>>,
    pub prev_version: Option<u32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeStatus {
    pub id: u64,
    pub stage: UpgradeStage,
    pub approval_count: u32,
    pub required_approvals: u32,
    pub target_version: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
// The `Up` prefix is intentional: it namespaces these storage keys to prevent
// collisions with other contracts sharing the same Soroban persistent storage.
#[allow(clippy::enum_variant_names)]
enum UpgradeKey {
    UpAdmin,
    UpApprovers,
    UpReqApprovals,
    UpNextPropId,
    UpCurrWasmHash,
    UpCurrVersion,
    UpProposal(u64),
}

pub struct UpgradeManager;

impl UpgradeManager {
    #[allow(deprecated)]
    pub fn init(env: Env, admin: Address, current_wasm_hash: BytesN<32>, required_approvals: u32) {
        if env.storage().persistent().has(&UpgradeKey::UpAdmin) {
            panic_with_error!(&env, UpgradeError::AlreadyInitialized);
        }
        if required_approvals == 0 {
            panic_with_error!(&env, UpgradeError::InvalidThreshold);
        }

        let mut approvers = Vec::new(&env);
        approvers.push_back(admin.clone());

        env.storage().persistent().set(&UpgradeKey::UpAdmin, &admin);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpApprovers, &approvers);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpReqApprovals, &required_approvals);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpNextPropId, &1u64);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrWasmHash, &current_wasm_hash);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrVersion, &0u32);

        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("up_init"), admin), required_approvals);
    }

    #[allow(deprecated)]
    pub fn add_approver(env: Env, caller: Address, approver: Address) {
        caller.require_auth();
        Self::assert_admin(&env, &caller);

        let mut approvers = Self::approvers(&env);
        if !approvers.contains(&approver) {
            approvers.push_back(approver.clone());
            env.storage()
                .persistent()
                .set(&UpgradeKey::UpApprovers, &approvers);
        }

        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("up_apadd"), caller, approver), ());
    }

    #[allow(deprecated)]
    pub fn remove_approver(env: Env, caller: Address, approver: Address) {
        caller.require_auth();
        Self::assert_admin(&env, &caller);

        let approvers = Self::approvers(&env);
        let mut updated = Vec::new(&env);
        for existing in approvers.iter() {
            if existing != approver {
                updated.push_back(existing);
            }
        }

        if updated.len() == approvers.len() {
            return;
        }
        if updated.is_empty() || updated.len() < Self::required_approvals(env.clone()) {
            panic_with_error!(&env, UpgradeError::InvalidThreshold);
        }

        env.storage()
            .persistent()
            .set(&UpgradeKey::UpApprovers, &updated);
        env.events()
            .publish((symbol_short!("up_aprm"), caller, approver), ());
    }

    pub fn upgrade_propose(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) -> u64 {
        caller.require_auth();
        Self::assert_admin(&env, &caller);

        let current_version = Self::current_version(env.clone());
        if new_version <= current_version {
            panic_with_error!(&env, UpgradeError::InvalidVersion);
        }

        let mut approvals = Vec::new(&env);
        approvals.push_back(caller.clone());

        let required = Self::required_approvals(env.clone());
        let stage = if approvals.len() >= required {
            UpgradeStage::Approved
        } else {
            UpgradeStage::Proposed
        };

        let id: u64 = env
            .storage()
            .persistent()
            .get(&UpgradeKey::UpNextPropId)
            .unwrap_or(1);
        let proposal = UpgradeProposal {
            id,
            proposer: caller.clone(),
            new_wasm_hash,
            new_version,
            approvals,
            stage,
            prev_wasm_hash: None,
            prev_version: None,
        };

        env.storage()
            .persistent()
            .set(&UpgradeKey::UpProposal(id), &proposal);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpNextPropId, &(id + 1));

        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("up_prop"), caller, id), new_version);
        id
    }

    #[allow(deprecated)]
    pub fn upgrade_approve(env: Env, caller: Address, proposal_id: u64) -> u32 {
        caller.require_auth();
        Self::assert_approver(&env, &caller);

        let mut proposal = Self::proposal(env.clone(), proposal_id);
        if proposal.stage != UpgradeStage::Proposed && proposal.stage != UpgradeStage::Approved {
            panic_with_error!(&env, UpgradeError::InvalidStatus);
        }
        if proposal.approvals.contains(&caller) {
            panic_with_error!(&env, UpgradeError::AlreadyApproved);
        }

        proposal.approvals.push_back(caller.clone());
        if proposal.approvals.len() >= Self::required_approvals(env.clone()) {
            proposal.stage = UpgradeStage::Approved;
        }
        let count = proposal.approvals.len();

        env.storage()
            .persistent()
            .set(&UpgradeKey::UpProposal(proposal_id), &proposal);
        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("up_appr"), caller, proposal_id), count);
        count
    }

    #[allow(deprecated)]
    pub fn upgrade_execute(env: Env, caller: Address, proposal_id: u64) {
        caller.require_auth();
        Self::assert_approver(&env, &caller);

        let mut proposal = Self::proposal(env.clone(), proposal_id);
        if proposal.stage != UpgradeStage::Approved {
            panic_with_error!(&env, UpgradeError::InvalidStatus);
        }

        let current_hash = Self::current_wasm_hash(env.clone());
        let current_version = Self::current_version(env.clone());
        proposal.prev_wasm_hash = Some(current_hash.clone());
        proposal.prev_version = Some(current_version);
        proposal.stage = UpgradeStage::Executed;

        #[cfg(not(any(test, feature = "testutils")))]
        env.deployer()
            .update_current_contract_wasm(proposal.new_wasm_hash.clone());

        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrWasmHash, &proposal.new_wasm_hash);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrVersion, &proposal.new_version);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpProposal(proposal_id), &proposal);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("up_exec"), caller, proposal_id),
            proposal.new_version,
        );
    }

    #[allow(deprecated)]
    pub fn upgrade_rollback(env: Env, caller: Address, proposal_id: u64) {
        caller.require_auth();
        Self::assert_admin(&env, &caller);

        let mut proposal = Self::proposal(env.clone(), proposal_id);
        if proposal.stage != UpgradeStage::Executed {
            panic_with_error!(&env, UpgradeError::InvalidStatus);
        }

        let prev_hash = proposal.prev_wasm_hash.clone().unwrap();
        let prev_version = proposal.prev_version.unwrap();

        #[cfg(not(any(test, feature = "testutils")))]
        env.deployer()
            .update_current_contract_wasm(prev_hash.clone());

        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrWasmHash, &prev_hash);
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpCurrVersion, &prev_version);

        proposal.stage = UpgradeStage::RolledBack;
        env.storage()
            .persistent()
            .set(&UpgradeKey::UpProposal(proposal_id), &proposal);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("up_roll"), caller, proposal_id),
            prev_version,
        );
    }

    pub fn upgrade_status(env: Env, proposal_id: u64) -> UpgradeStatus {
        let proposal = Self::proposal(env.clone(), proposal_id);
        UpgradeStatus {
            id: proposal.id,
            stage: proposal.stage,
            approval_count: proposal.approvals.len(),
            required_approvals: Self::required_approvals(env),
            target_version: proposal.new_version,
        }
    }

    pub fn current_wasm_hash(env: Env) -> BytesN<32> {
        env.storage()
            .persistent()
            .get(&UpgradeKey::UpCurrWasmHash)
            .unwrap()
    }

    pub fn current_version(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&UpgradeKey::UpCurrVersion)
            .unwrap_or(0)
    }

    fn required_approvals(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&UpgradeKey::UpReqApprovals)
            .unwrap_or(0)
    }

    fn proposal(env: Env, proposal_id: u64) -> UpgradeProposal {
        env.storage()
            .persistent()
            .get(&UpgradeKey::UpProposal(proposal_id))
            .unwrap_or_else(|| panic_with_error!(&env, UpgradeError::ProposalNotFound))
    }

    fn approvers(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&UpgradeKey::UpApprovers)
            .unwrap_or_else(|| Vec::new(env))
    }

    fn assert_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&UpgradeKey::UpAdmin)
            .unwrap();
        if *caller != admin {
            panic_with_error!(env, UpgradeError::NotAuthorized);
        }
    }

    fn assert_approver(env: &Env, caller: &Address) {
        if !Self::approvers(env).contains(caller) {
            panic_with_error!(env, UpgradeError::NotAuthorized);
        }
    }
}
