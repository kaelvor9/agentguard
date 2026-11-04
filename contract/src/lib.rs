//! AgentGuard vault: an AI agent can *request* payments, but only this contract moves money.
//!
//! Roles (maker-checker):
//! - owner:    the human. Sets policy, approves/rejects held payments, pauses, withdraws.
//! - agent:    the AI. Can only call `request_payment` (via a function-call key).
//! - guardian: the risk scorer. Can only call `submit_risk` (via a function-call key).
use near_sdk::json_types::U64;
use near_sdk::serde_json::json;
use near_sdk::store::{IterableMap, Vector};
use near_sdk::{
    AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise, assert_one_yocto, env, near,
    require,
};

const NS_PER_DAY: u64 = 86_400 * 1_000_000_000;
/// Held payments must be approved within 24 hours or they expire.
const APPROVAL_WINDOW_NS: u64 = NS_PER_DAY;
const MAX_MEMO_LEN: usize = 280;
const MAX_REASON_LEN: usize = 280;
/// Caps how many unresolved requests can exist, so a hijacked agent can't spam storage.
const MAX_OPEN_REQUESTS: u32 = 20;

#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    Payees,
    Requests,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    /// Hard cap for a single payment. Anything above is rejected outright.
    pub per_tx_limit: NearToken,
    /// Hard cap for the total paid out per UTC day.
    pub daily_limit: NearToken,
    /// Payments above this amount need the owner's approval.
    pub approval_threshold: NearToken,
    /// Risk score (0-100) at or above which a payment is held for approval.
    pub risk_hold: u8,
    /// Risk score (0-100) at or above which a payment is rejected.
    pub risk_block: u8,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    PendingRisk,
    NeedsApproval,
    Executed,
    Rejected,
    Expired,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct PaymentRequest {
    pub id: U64,
    pub to: AccountId,
    pub amount: NearToken,
    pub memo: String,
    pub status: Status,
    pub risk_score: Option<u8>,
    pub risk_reason: Option<String>,
    /// Why the vault decided what it did (e.g. "over per-transaction limit").
    pub decision: Option<String>,
    pub created_at: U64,
}

#[near(serializers = [json])]
pub struct Config {
    pub owner: AccountId,
    pub agent: AccountId,
    pub guardian: AccountId,
    pub paused: bool,
    pub policy: Policy,
}

#[near(serializers = [json])]
pub struct Spending {
    pub spent_today: NearToken,
    pub remaining_today: NearToken,
    pub available_balance: NearToken,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Vault {
    owner: AccountId,
    agent: AccountId,
    guardian: AccountId,
    policy: Policy,
    paused: bool,
    payees: IterableMap<AccountId, String>,
    requests: Vector<PaymentRequest>,
    open_requests: u32,
    spent_today: NearToken,
    day_index: u64,
}

#[near]
impl Vault {
    /// Called once, by the vault account itself, right after deploying.
    #[init]
    #[private]
    pub fn new(owner: AccountId, agent: AccountId, guardian: AccountId, policy: Policy) -> Self {
        Self::validate_policy(&policy);
        Self {
            owner,
            agent,
            guardian,
            policy,
            paused: false,
            payees: IterableMap::new(StorageKey::Payees),
            requests: Vector::new(StorageKey::Requests),
            open_requests: 0,
            spent_today: NearToken::from_yoctonear(0),
            day_index: env::block_timestamp() / NS_PER_DAY,
        }
    }

    // ---------------------------------------------------------------- agent

    /// The agent asks for a payment. Hard rules are checked immediately; anything that passes
    /// waits for the guardian's risk score. Returns the request id.
    pub fn request_payment(&mut self, to: AccountId, amount: NearToken, memo: String) -> U64 {
        require!(env::predecessor_account_id() == self.agent, "Only the agent can request payments");
        require!(memo.len() <= MAX_MEMO_LEN, "Memo too long");
        require!(amount.as_yoctonear() > 0, "Amount must be greater than zero");
        require!(self.open_requests < MAX_OPEN_REQUESTS, "Too many open requests");
        self.roll_day();

        let id = self.requests.len() as u64;
        let mut request = PaymentRequest {
            id: U64(id),
            to,
            amount,
            memo,
            status: Status::PendingRisk,
            risk_score: None,
            risk_reason: None,
            decision: None,
            created_at: U64(env::block_timestamp()),
        };

        let hard_rejection = if self.paused {
            Some("vault is paused")
        } else if amount > self.policy.per_tx_limit {
            Some("over per-transaction limit")
        } else if self.spent_today.saturating_add(amount) > self.policy.daily_limit {
            Some("over daily limit")
        } else {
            None
        };

        self.emit("payment_requested", &request);
        match hard_rejection {
            Some(reason) => {
                request.status = Status::Rejected;
                request.decision = Some(reason.to_string());
                self.emit("payment_rejected", &request);
            }
            None => self.open_requests += 1,
        }
        self.requests.push(request);
        U64(id)
    }

    // ------------------------------------------------------------- guardian

    /// The guardian posts the AI risk score. The vault applies the policy and pays or holds.
    pub fn submit_risk(&mut self, id: U64, score: u8, reason: String) -> Status {
        require!(env::predecessor_account_id() == self.guardian, "Only the guardian can submit risk");
        require!(score <= 100, "Score must be 0-100");
        require!(reason.len() <= MAX_REASON_LEN, "Reason too long");
        self.roll_day();

        let mut request = self.get_request_or_panic(id.0);
        require!(request.status == Status::PendingRisk, "Request is not awaiting a risk score");
        request.risk_score = Some(score);
        request.risk_reason = Some(reason);

        let is_payee = self.payees.contains_key(&request.to);
        let decision: (Status, &str) = if self.paused {
            (Status::Rejected, "vault is paused")
        } else if score >= self.policy.risk_block {
            (Status::Rejected, "risk score at or above block level")
        } else if !is_payee {
            (Status::NeedsApproval, "payee not on allowlist")
        } else if score >= self.policy.risk_hold {
            (Status::NeedsApproval, "risk score at or above hold level")
        } else if request.amount > self.policy.approval_threshold {
            (Status::NeedsApproval, "amount above approval threshold")
        } else if self.spent_today.saturating_add(request.amount) > self.policy.daily_limit {
            (Status::Rejected, "over daily limit")
        } else if request.amount > self.available_balance() {
            (Status::Rejected, "insufficient vault balance")
        } else {
            (Status::Executed, "within policy")
        };

        request.status = decision.0.clone();
        request.decision = Some(decision.1.to_string());
        match decision.0 {
            Status::Executed => self.execute(&mut request),
            Status::Rejected => {
                self.open_requests -= 1;
                self.emit("payment_rejected", &request);
            }
            _ => self.emit("payment_held", &request),
        }
        self.store_request(request);
        decision.0
    }

    // ---------------------------------------------------------------- owner

    /// Release a held payment. Needs 1 yoctoNEAR attached, which forces a full-access key.
    #[payable]
    pub fn approve(&mut self, id: U64) -> Status {
        self.assert_owner();
        self.roll_day();
        let mut request = self.get_request_or_panic(id.0);
        require!(request.status == Status::NeedsApproval, "Request is not awaiting approval");

        if env::block_timestamp() > request.created_at.0 + APPROVAL_WINDOW_NS {
            request.status = Status::Expired;
            request.decision = Some("approval window expired".to_string());
            self.open_requests -= 1;
            self.emit("payment_expired", &request);
        } else {
            require!(request.amount <= self.available_balance(), "Insufficient vault balance");
            request.status = Status::Executed;
            request.decision = Some("approved by owner".to_string());
            self.execute(&mut request);
        }
        let status = request.status.clone();
        self.store_request(request);
        status
    }

    #[payable]
    pub fn reject(&mut self, id: U64) {
        self.assert_owner();
        let mut request = self.get_request_or_panic(id.0);
        require!(
            request.status == Status::NeedsApproval || request.status == Status::PendingRisk,
            "Request is already resolved"
        );
        request.status = Status::Rejected;
        request.decision = Some("rejected by owner".to_string());
        self.open_requests -= 1;
        self.emit("payment_rejected", &request);
        self.store_request(request);
    }

    #[payable]
    pub fn set_policy(&mut self, policy: Policy) {
        self.assert_owner();
        Self::validate_policy(&policy);
        self.policy = policy;
    }

    #[payable]
    pub fn add_payee(&mut self, account: AccountId, label: String) {
        self.assert_owner();
        require!(label.len() <= 64, "Label too long");
        self.payees.insert(account, label);
    }

    #[payable]
    pub fn remove_payee(&mut self, account: AccountId) {
        self.assert_owner();
        self.payees.remove(&account);
    }

    /// Kill switch: every request and pending decision is rejected while paused.
    #[payable]
    pub fn pause(&mut self) {
        self.assert_owner();
        self.paused = true;
        env::log_str("EVENT_JSON:{\"standard\":\"agentguard\",\"version\":\"1.0.0\",\"event\":\"paused\",\"data\":[]}");
    }

    #[payable]
    pub fn unpause(&mut self) {
        self.assert_owner();
        self.paused = false;
        env::log_str("EVENT_JSON:{\"standard\":\"agentguard\",\"version\":\"1.0.0\",\"event\":\"unpaused\",\"data\":[]}");
    }

    /// Replace a compromised agent or guardian account.
    #[payable]
    pub fn set_agent(&mut self, agent: AccountId) {
        self.assert_owner();
        self.agent = agent;
    }

    #[payable]
    pub fn set_guardian(&mut self, guardian: AccountId) {
        self.assert_owner();
        self.guardian = guardian;
    }

    /// Owner pulls funds out of the vault.
    #[payable]
    pub fn withdraw(&mut self, amount: NearToken) -> Promise {
        self.assert_owner();
        require!(amount <= self.available_balance(), "Insufficient vault balance");
        Promise::new(self.owner.clone()).transfer(amount)
    }

    // ---------------------------------------------------------------- views

    pub fn get_config(&self) -> Config {
        Config {
            owner: self.owner.clone(),
            agent: self.agent.clone(),
            guardian: self.guardian.clone(),
            paused: self.paused,
            policy: self.policy.clone(),
        }
    }

    pub fn get_payees(&self) -> Vec<(AccountId, String)> {
        self.payees.iter().map(|(a, l)| (a.clone(), l.clone())).collect()
    }

    pub fn get_request(&self, id: U64) -> Option<PaymentRequest> {
        self.requests.get(id.0 as u32).cloned()
    }

    /// Newest first. `from_index` counts back from the newest request.
    pub fn list_requests(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<PaymentRequest> {
        let len = self.requests.len();
        let skip = from_index.unwrap_or(0).min(len);
        let limit = limit.unwrap_or(20).min(100);
        (0..len - skip)
            .rev()
            .take(limit as usize)
            .filter_map(|i| self.requests.get(i).cloned())
            .collect()
    }

    pub fn get_spending(&self) -> Spending {
        let today = env::block_timestamp() / NS_PER_DAY;
        let spent = if today == self.day_index { self.spent_today } else { NearToken::from_yoctonear(0) };
        Spending {
            spent_today: spent,
            remaining_today: self.policy.daily_limit.saturating_sub(spent),
            available_balance: self.available_balance(),
        }
    }

    // -------------------------------------------------------------- helpers

    fn assert_owner(&self) {
        assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "Only the owner can do this");
    }

    fn validate_policy(policy: &Policy) {
        require!(policy.risk_hold <= policy.risk_block, "risk_hold must be <= risk_block");
        require!(policy.risk_block <= 100, "risk_block must be <= 100");
        require!(policy.per_tx_limit <= policy.daily_limit, "per_tx_limit must be <= daily_limit");
    }

    /// Resets the daily counter when a new UTC day starts.
    fn roll_day(&mut self) {
        let today = env::block_timestamp() / NS_PER_DAY;
        if today != self.day_index {
            self.day_index = today;
            self.spent_today = NearToken::from_yoctonear(0);
        }
    }

    /// Balance the vault can pay out without touching the NEAR it must keep for storage.
    fn available_balance(&self) -> NearToken {
        let storage_cost = env::storage_byte_cost().saturating_mul(env::storage_usage() as u128);
        env::account_balance().saturating_sub(storage_cost)
    }

    fn execute(&mut self, request: &mut PaymentRequest) {
        self.spent_today = self.spent_today.saturating_add(request.amount);
        self.open_requests -= 1;
        self.emit("payment_executed", request);
        Promise::new(request.to.clone()).transfer(request.amount).detach();
    }

    fn get_request_or_panic(&self, id: u64) -> PaymentRequest {
        self.requests
            .get(id as u32)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Request not found"))
    }

    fn store_request(&mut self, request: PaymentRequest) {
        let id = request.id.0 as u32;
        self.requests.replace(id, request);
    }

    /// NEP-297 event log, readable from any explorer.
    fn emit(&self, event: &str, request: &PaymentRequest) {
        let log = json!({
            "standard": "agentguard",
            "version": "1.0.0",
            "event": event,
            "data": [request],
        });
        env::log_str(&format!("EVENT_JSON:{}", log));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    fn acc(name: &str) -> AccountId {
        name.parse().unwrap()
    }

    fn near(n: u128) -> NearToken {
        NearToken::from_near(n)
    }

    fn ctx(caller: &str, deposit: u128) {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("vault.testnet"))
            .predecessor_account_id(acc(caller))
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .account_balance(near(100))
            .block_timestamp(1_000 * NS_PER_DAY);
        testing_env!(b.build());
    }

    fn setup() -> Vault {
        ctx("vault.testnet", 0);
        let mut v = Vault::new(
            acc("owner.testnet"),
            acc("agent.testnet"),
            acc("guardian.testnet"),
            Policy {
                per_tx_limit: near(5),
                daily_limit: near(10),
                approval_threshold: near(3),
                risk_hold: 60,
                risk_block: 90,
            },
        );
        ctx("owner.testnet", 1);
        v.add_payee(acc("shop.testnet"), "API credits".into());
        v
    }

    fn request(v: &mut Vault, to: &str, amount: u128) -> U64 {
        ctx("agent.testnet", 0);
        v.request_payment(acc(to), near(amount), "invoice".into())
    }

    fn score(v: &mut Vault, id: U64, s: u8) -> Status {
        ctx("guardian.testnet", 0);
        v.submit_risk(id, s, "test".into())
    }

    fn status(v: &Vault, id: U64) -> Status {
        v.get_request(id).unwrap().status
    }

    #[test]
    fn low_risk_allowlisted_payment_executes() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 1);
        assert_eq!(score(&mut v, id, 10), Status::Executed);
        assert_eq!(v.get_spending().spent_today, near(1));
    }

    #[test]
    fn over_per_tx_limit_is_rejected_immediately() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 50);
        assert_eq!(status(&v, id), Status::Rejected);
    }

    #[test]
    fn high_risk_is_rejected() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 1);
        assert_eq!(score(&mut v, id, 95), Status::Rejected);
    }

    #[test]
    fn medium_risk_is_held() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 1);
        assert_eq!(score(&mut v, id, 70), Status::NeedsApproval);
    }

    #[test]
    fn unknown_payee_is_held() {
        let mut v = setup();
        let id = request(&mut v, "stranger.testnet", 1);
        assert_eq!(score(&mut v, id, 5), Status::NeedsApproval);
    }

    #[test]
    fn above_threshold_is_held_then_approved() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 4);
        assert_eq!(score(&mut v, id, 5), Status::NeedsApproval);
        ctx("owner.testnet", 1);
        assert_eq!(v.approve(id), Status::Executed);
        assert_eq!(v.get_spending().spent_today, near(4));
    }

    #[test]
    fn daily_limit_rejects_new_requests() {
        let mut v = setup();
        for _ in 0..3 {
            let id = request(&mut v, "shop.testnet", 3);
            assert_eq!(score(&mut v, id, 5), Status::Executed);
        }
        let id = request(&mut v, "shop.testnet", 3);
        assert_eq!(status(&v, id), Status::Rejected);
    }

    #[test]
    fn paused_vault_rejects_everything() {
        let mut v = setup();
        ctx("owner.testnet", 1);
        v.pause();
        let id = request(&mut v, "shop.testnet", 1);
        assert_eq!(status(&v, id), Status::Rejected);
    }

    #[test]
    #[should_panic(expected = "Only the agent")]
    fn only_agent_can_request() {
        let mut v = setup();
        ctx("guardian.testnet", 0);
        v.request_payment(acc("shop.testnet"), near(1), "x".into());
    }

    #[test]
    #[should_panic(expected = "Only the guardian")]
    fn agent_cannot_score_itself() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 1);
        ctx("agent.testnet", 0);
        v.submit_risk(id, 0, "trust me".into());
    }

    #[test]
    #[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
    fn approve_needs_one_yocto() {
        let mut v = setup();
        let id = request(&mut v, "shop.testnet", 4);
        score(&mut v, id, 5);
        ctx("owner.testnet", 0);
        v.approve(id);
    }

    #[test]
    fn list_is_newest_first() {
        let mut v = setup();
        request(&mut v, "shop.testnet", 1);
        request(&mut v, "shop.testnet", 2);
        let list = v.list_requests(None, None);
        assert_eq!(list[0].amount, near(2));
        assert_eq!(list.len(), 2);
    }
}
