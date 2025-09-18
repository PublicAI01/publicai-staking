use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::collections::UnorderedMap;
use near_sdk::json_types::U128;
use near_sdk::{
    assert_one_yocto, env, log, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise,
    PromiseOrValue,
};
use serde_json::json;
const CURRENT_STATE_VERSION: u32 = 1;
const NO_DEPOSIT: NearToken = NearToken::from_near(0);
const OUTER_UPGRADE_GAS: Gas = Gas::from_tgas(20);
// Constants
const AAR: u128 = 800; // Annualized Annual Rate (8%)
const SECONDS_IN_A_YEAR: u128 = 365 * 24 * 60 * 60; // Number of seconds in a year
const WEEK: u64 = 7 * 24 * 60 * 60; // Number of seconds in a week
const NANOSECONDS: u64 = 1_000_000_000; // Nanoseconds to seconds
const AAR_BASE: u128 = 10000;
const MAX_TOTAL_REWARD: u128 = 100000000_000_000_000_000_000_000;
const MAX_LOCK_DURATION: u64 = 4 * WEEK;
const AAR_EARLY: [u128; 5] = [50000, 50000, 10000, 5000, 5000]; // Week 1,2,3,4,5 AAR
/// Struct for storing staking information
#[near(serializers = [json, borsh])]
pub struct StakeInfo {
    amount: u128,             // The principal amount staked by the user
    accumulated_reward: u128, // Accumulated interest rewards
    first_stake_time: u64,    // Time of first stake
    start_time: u64,          // Timestamp when staking began
}

#[near(serializers = [json, borsh])]
pub enum UserOperationState {
    Idle,
    Staking,
    Unstaking,
}
/// Main contract struct
#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct StakingContract {
    owner_id: AccountId,                                      // Contract owner
    token_contract: AccountId,                                // NEP-141 token contract address
    staked_balances: UnorderedMap<AccountId, StakeInfo>,      // User staking information
    user_states: UnorderedMap<AccountId, UserOperationState>, // User operation state
    stake_start_time: u64,                                    // Start time of stake
    lock_duration: u64,                                       // Lock duration
    stake_paused: bool,                                       // Pause stake
    stake_end_time: u64, // Stake end time,after this time, there will be no rewards for stake,0 means no end time.
    total_staked: u128,  // Total amount staked
    total_claimed_reward: u128, // Total amount of claimed reward
    total_reward: u128,  // Total amount of reward
}

#[near]
impl StakingContract {
    /// Initialize the contract
    #[init]
    pub fn new(owner_id: AccountId, token_contract: AccountId, total_reward: U128) -> Self {
        assert!(!env::state_exists(), "Already initialized");
        let reward = total_reward.0;
        assert!(reward > 0, "Total reward should gt 0");
        let current_time = env::block_timestamp() / NANOSECONDS;
        Self {
            owner_id,
            token_contract,
            staked_balances: UnorderedMap::new(b"s".to_vec()),
            user_states: UnorderedMap::new(b"user_states".to_vec()),
            stake_paused: false,
            stake_start_time: current_time,
            lock_duration: 2 * WEEK, // Lock 2 week on default
            stake_end_time: 0,
            total_staked: 0,
            total_claimed_reward: 0,
            total_reward: reward,
        }
    }

    /// Pause or start stake (only callable by the owner).
    /// - `pause`: If true, staking is paused, if false, staking is started.
    #[payable]
    pub fn pause_stake(&mut self, pause: bool) {
        assert_one_yocto();
        assert_eq!(
            self.owner_id,
            env::predecessor_account_id(),
            "Only the owner can pause or start stake."
        );
        self.stake_paused = pause;
        env::log_str(&format!("Stake paused updated to {}", self.stake_paused));
    }

    /// Set lock duration (only callable by the owner).
    /// - `lock_duration`: Lock duration.
    #[payable]
    pub fn set_lock_duration(&mut self, lock_duration: u64) {
        assert_one_yocto();
        assert_eq!(
            self.owner_id,
            env::predecessor_account_id(),
            "Only the owner can set lock duration."
        );
        require!(
            lock_duration <= MAX_LOCK_DURATION,
            "Cannot exceed MAX_LOCK_DURATION"
        );
        self.lock_duration = lock_duration;
        env::log_str(&format!("Lock duration updated to {}", self.lock_duration));
    }

    #[payable]
    pub fn update_owner(&mut self, new_owner: AccountId) -> bool {
        assert_one_yocto();
        require!(
            env::predecessor_account_id() == self.owner_id,
            "Owner's method"
        );
        require!(!new_owner.as_str().is_empty(), "New owner cannot be empty");
        log!("Owner updated from {} to {}", self.owner_id, new_owner);
        self.owner_id = new_owner;
        true
    }
    /// Set stake end time (only callable by the owner).
    /// - `end_time`: End time timestamp.
    #[payable]
    pub fn set_stake_end_time(&mut self, end_time: u64) {
        assert_one_yocto();
        assert_eq!(
            self.owner_id,
            env::predecessor_account_id(),
            "Only the owner can set end time."
        );
        if end_time == 0 {
            // No end time
            assert_eq!(self.stake_paused, false, "Need to start stake first.");
        } else {
            assert_eq!(self.stake_paused, true, "Need to pause stake first.");
        }
        self.stake_end_time = end_time;
        env::log_str(&format!(
            "Stake end time updated to {}",
            self.stake_end_time
        ));
    }

    /// Set total reward (only callable by the owner).
    /// - `total_reward`: Total reward.
    #[payable]
    pub fn set_total_reward(&mut self, total_reward: U128) {
        assert_one_yocto();
        assert_eq!(
            self.owner_id,
            env::predecessor_account_id(),
            "Only the owner can set total reward."
        );
        let reward = total_reward.0;
        assert!(reward > 0, "Total reward should gt 0.");
        assert!(
            reward <= MAX_TOTAL_REWARD,
            "Total reward should le MAX_TOTAL_REWARD"
        );
        self.total_reward = reward;
        env::log_str(&format!("Total reward updated to {}", self.total_reward));
    }

    /// Unstake all principal and rewards
    #[payable]
    pub fn unstake(&mut self) -> Promise {
        assert_one_yocto();
        let account_id = env::predecessor_account_id();
        let mut stake_info = self
            .staked_balances
            .get(&account_id)
            .expect("No stake found for this account");

        match self.user_states.get(&account_id) {
            Some(UserOperationState::Idle) | None => {
                // pass
                self.user_states
                    .insert(&account_id, &UserOperationState::Unstaking);
                env::log_str("Unstake operation started.");
            }
            Some(UserOperationState::Staking) => {
                env::panic_str("Cannot unstake while staking is in progress.");
            }
            Some(UserOperationState::Unstaking) => {
                env::panic_str("Unstake operation already in progress.");
            }
        }
        // Calculate the time difference and accumulated rewards
        let current_time = env::block_timestamp() / NANOSECONDS; // Convert nanoseconds to seconds
        let reward_end_time = if self.stake_end_time == 0 {
            current_time
        } else {
            std::cmp::min(current_time, self.stake_end_time)
        };

        let start_time = if reward_end_time >= stake_info.start_time {
            stake_info.start_time
        } else {
            reward_end_time
        };

        // Update accumulated rewards
        let reward = self.calculate_reward(stake_info.amount, reward_end_time, start_time);
        let after_total_claimed_reward = self.total_claimed_reward + reward;
        let mut claim_reward = 0;
        // The user can only claim the portion that does not exceed the total reward.
        if after_total_claimed_reward >= self.total_reward {
            if self.total_reward >= self.total_claimed_reward {
                claim_reward = self.total_reward - self.total_claimed_reward;
            }
        } else {
            claim_reward = reward;
        }
        let before_accumulated_reward = stake_info.accumulated_reward;
        stake_info.accumulated_reward += claim_reward;

        let mut reward_amount = stake_info.accumulated_reward;
        // Total payout = principal + accumulated rewards
        // If the lock-up period is not exceeded, only the principal will be returned.
        let total_payout = if current_time > stake_info.first_stake_time + self.lock_duration {
            stake_info.amount + stake_info.accumulated_reward
        } else {
            reward_amount = 0;
            stake_info.amount
        };

        // Remove staking record
        self.staked_balances.remove(&account_id);

        // Transfer principal and rewards to the user
        Promise::new(self.token_contract.clone())
            .function_call(
                "ft_transfer".to_string(),
                serde_json::json!({
                    "receiver_id": account_id,
                    "amount": total_payout.to_string(),
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(1), // Attach 1 yoctoNEAR
                Gas::from_gas(20_000_000_000_000),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_gas(5_000_000_000_000))
                    .on_ft_transfer_then_remove(
                        account_id,
                        stake_info.amount,
                        reward_amount,
                        stake_info.first_stake_time,
                        stake_info.start_time,
                        before_accumulated_reward,
                    ),
            )
    }

    /// Callback: After ft_transfer, only then remove staking record.
    #[private]
    pub fn on_ft_transfer_then_remove(
        &mut self,
        account_id: AccountId,
        stake_amount: u128,
        reward_amount: u128,
        first_stake_time: u64,
        start_time: u64,
        before_reward_amount: u128,
        #[callback_result] call_result: Result<(), near_sdk::PromiseError>,
    ) -> bool {
        match call_result {
            Ok(()) => {
                self.total_staked -= stake_amount;
                self.total_claimed_reward += reward_amount;
                self.user_states
                    .insert(&account_id, &UserOperationState::Idle);
                true
            }
            Err(_) => {
                let stake_info = StakeInfo {
                    amount: stake_amount,
                    accumulated_reward: before_reward_amount,
                    first_stake_time,
                    start_time,
                };
                self.staked_balances.insert(&account_id, &stake_info);
                self.user_states
                    .insert(&account_id, &UserOperationState::Idle);
                false
            }
        }
    }

    /// Query staking information for a specific user
    pub fn get_stake_info(&self, account_id: AccountId) -> Option<StakeInfo> {
        if let Some(mut stake_info) = self.staked_balances.get(&account_id) {
            // Calculate the time difference
            let current_time = env::block_timestamp() / NANOSECONDS; // Convert nanoseconds to seconds
            let reward_end_time = if self.stake_end_time == 0 {
                current_time
            } else {
                std::cmp::min(current_time, self.stake_end_time)
            };

            let start_time = if reward_end_time >= stake_info.start_time {
                stake_info.start_time
            } else {
                reward_end_time
            };

            // Calculate real-time rewards
            let reward = self.calculate_reward(stake_info.amount, reward_end_time, start_time);

            // Update the accumulated reward (real-time)
            stake_info.accumulated_reward += reward;

            // Return the updated stake info with real-time rewards
            Some(stake_info)
        } else {
            None
        }
    }

    /// Calculate rewards based on staking amount and duration
    fn calculate_reward(&self, amount: u128, current_time: u64, start_time: u64) -> u128 {
        let mut reward = 0u128;
        // Reward formula: Principal * AAR * duration / (SECONDS_IN_A_YEAR * 10000)
        for (index, aar) in AAR_EARLY.iter().enumerate() {
            let aar_start_at = self.stake_start_time + (index as u64 * WEEK);
            let aar_end_at = self.stake_start_time + ((index + 1) as u64 * WEEK);
            // Skip if the entire interval is outside the range
            if current_time < aar_start_at || start_time >= aar_end_at {
                continue;
            }
            let reward_duration = if start_time >= aar_start_at {
                if current_time <= aar_end_at {
                    current_time - start_time
                } else {
                    aar_end_at - start_time
                }
            } else {
                if current_time <= aar_end_at {
                    current_time - aar_start_at
                } else {
                    aar_end_at - aar_start_at
                }
            };
            reward += amount * aar * (reward_duration as u128);
        }
        let last_interval_end = self.stake_start_time + (AAR_EARLY.len() as u64 * WEEK);
        if current_time >= last_interval_end {
            let reward_duration = if start_time >= last_interval_end {
                current_time - start_time
            } else {
                current_time - last_interval_end
            };
            reward += amount * AAR * (reward_duration as u128);
        }
        reward / (SECONDS_IN_A_YEAR * AAR_BASE)
    }

    /// Query total stake
    pub fn get_total_stake(&self) -> u128 {
        self.total_staked
    }

    /// Query total claimed reward
    pub fn get_total_claimed_reward(&self) -> u128 {
        self.total_claimed_reward
    }

    /// Only owner can call. Transfer `amount` of given token to `to`.
    #[payable]
    pub fn withdraw_token(&mut self, amount: U128) -> Promise {
        assert_one_yocto();
        // Ensure only owner can call
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Only the owner can withdraw tokens"
        );

        assert_eq!(self.stake_paused, true, "Stake should paused");

        Promise::new(self.token_contract.clone())
            .function_call(
                "ft_balance_of".to_string(),
                serde_json::json!({
                    "account_id": env::current_account_id()
                })
                .to_string()
                .into_bytes(),
                NearToken::from_near(0),
                Gas::from_gas(10_000_000_000_000),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_gas(30_000_000_000_000))
                    .on_check_balance_then_withdraw(
                        self.token_contract.clone(),
                        self.owner_id.clone(),
                        amount,
                    ),
            )
    }

    #[private]
    pub fn on_check_balance_then_withdraw(
        &self,
        token_contract: AccountId,
        to: AccountId,
        amount: U128,
        #[callback_result] call_result: Result<Option<U128>, near_sdk::PromiseError>,
    ) -> Promise {
        let balance = match call_result {
            Ok(Some(b)) => b.0,
            _ => env::panic_str("Failed to get token balance"),
        };
        let mut available = 0;
        let mut frozen = self.total_staked;
        if self.total_reward >= self.total_claimed_reward {
            frozen += self.total_reward - self.total_claimed_reward;
        }

        if balance > frozen {
            available = balance - frozen;
        }
        assert!(
            amount.0 <= available,
            "Not enough token balance to withdraw"
        );

        Promise::new(token_contract).function_call(
            "ft_transfer".to_string(),
            serde_json::json!({
                "receiver_id": to,
                "amount": amount,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_gas(10_000_000_000_000),
        )
    }

    #[private]
    #[init(ignore_state)]
    #[allow(unused_variables)]
    pub fn migrate(from_version: u32) -> Self {
        env::state_read().unwrap_or_else(|| env::panic_str("ERR_FAILED_TO_READ_STATE"))
    }

    pub fn update_contract(&self) {
        // Ensure only owner can call
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Only the owner can upgrade"
        );

        // Receive the code directly from the input to avoid the
        // GAS overhead of deserializing parameters
        let code = env::input().unwrap_or_else(|| env::panic_str("ERR_NO_INPUT"));
        // Deploy the contract code.
        let promise_id = env::promise_batch_create(&env::current_account_id());
        env::promise_batch_action_deploy_contract(promise_id, &code);
        // Call promise to migrate the state.
        // Batched together to fail upgrade if migration fails.
        env::promise_batch_action_function_call(
            promise_id,
            "migrate",
            &json!({ "from_version": CURRENT_STATE_VERSION })
                .to_string()
                .into_bytes(),
            NO_DEPOSIT,
            env::prepaid_gas()
                .saturating_sub(env::used_gas())
                .saturating_sub(OUTER_UPGRADE_GAS),
        );
        env::promise_return(promise_id);
    }

    /// Query owner
    pub fn owner(&self) -> AccountId {
        self.owner_id.clone()
    }

    /// Query aar
    pub fn get_aar(&self) -> [u128; 5] {
        AAR_EARLY
    }

    /// Query lock duration
    pub fn get_lock_duration(&self) -> u64 {
        self.lock_duration
    }

    pub fn search_stake_infos(
        &self,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Vec<(AccountId, StakeInfo)> {
        let start = offset.unwrap_or(0);
        let l = limit.unwrap_or(50);
        self.staked_balances
            .iter()
            .skip(start as usize)
            .take(l as usize)
            .collect()
    }
}

/// Implementation of NEP-141 `ft_on_transfer` method
#[near]
impl FungibleTokenReceiver for StakingContract {
    /// Handle token transfers for staking
    fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        // Ensure that the token being transferred is the one specified in the contract
        assert_eq!(
            env::predecessor_account_id(),
            self.token_contract,
            "Only the specified token can be staked"
        );

        assert_eq!(self.stake_paused, false, "Stake paused");

        match self.user_states.get(&sender_id) {
            Some(UserOperationState::Idle) | None => {
                self.user_states
                    .insert(&sender_id, &UserOperationState::Staking);
                env::log_str("Stake operation started.");
            }
            Some(UserOperationState::Staking) => {
                env::panic_str("Stake operation already in progress.");
            }
            Some(UserOperationState::Unstaking) => {
                env::panic_str("Cannot stake while unstake is in progress.");
            }
        }
        // Get the current timestamp
        let current_time = env::block_timestamp() / NANOSECONDS; // Convert nanoseconds to seconds

        // Update or create the user's staking record
        let mut stake_info = self.staked_balances.get(&sender_id).unwrap_or(StakeInfo {
            amount: 0,
            accumulated_reward: 0,
            first_stake_time: current_time,
            start_time: current_time,
        });

        // Update accumulated rewards
        let reward = self.calculate_reward(stake_info.amount, current_time, stake_info.start_time);
        stake_info.accumulated_reward += reward;

        // Update principal and timestamp
        stake_info.amount += amount.0;
        stake_info.start_time = current_time;

        self.staked_balances.insert(&sender_id, &stake_info);

        self.total_staked += amount.0;

        self.user_states
            .insert(&sender_id, &UserOperationState::Idle);
        // Return 0 to indicate the transfer was successfully handled
        PromiseOrValue::Value(U128(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::json_types::U128;
    use near_sdk::test_utils::accounts;
    use near_sdk::{test_utils::VMContextBuilder, testing_env, AccountId};

    const TOKEN_CONTRACT: &str = "token.testnet";

    /// Helper function to create a mock context
    fn get_context(
        predecessor: AccountId,
        attached_deposit: u128,
        block_timestamp: u64,
    ) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .predecessor_account_id(predecessor) // The account that sends the call (e.g., the token contract)
            .attached_deposit(NearToken::from_yoctonear(attached_deposit)) // The deposit attached with the call
            .block_timestamp(block_timestamp); // Set the block timestamp
        builder
    }

    #[test]
    fn test_contract_initialization() {
        // Set up the testing environment
        let context = get_context(accounts(0), 0, 0);
        testing_env!(context.build());

        // Initialize the contract
        let token_contract: AccountId = TOKEN_CONTRACT.parse().unwrap();
        let contract =
            StakingContract::new(accounts(0), token_contract.clone(), U128(1_000_000u128));

        // Check initialization
        assert_eq!(contract.owner_id, accounts(0));
        assert_eq!(contract.token_contract, token_contract);
    }

    #[test]
    fn test_staking() {
        // Set up the testing environment
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, 0);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate a user staking tokens via ft_on_transfer
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);

        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        // Check if the user's staking record is updated
        let stake_info = contract.get_stake_info(sender_id).unwrap();
        assert_eq!(stake_info.amount, stake_amount.0);
        assert_eq!(stake_info.accumulated_reward, 0);
    }

    #[test]
    fn test_multiple_staking() {
        // Set up the testing environment
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, 0);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate a user staking tokens multiple times
        let sender_id = accounts(1);
        let first_stake_amount = U128(1_000_000);
        let second_stake_amount = U128(500_000);

        contract.ft_on_transfer(sender_id.clone(), first_stake_amount, "".to_string());
        contract.ft_on_transfer(sender_id.clone(), second_stake_amount, "".to_string());

        // Check if the user's staking record is updated
        let stake_info = contract.get_stake_info(sender_id).unwrap();
        assert_eq!(
            stake_info.amount,
            first_stake_amount.0 + second_stake_amount.0
        );
        assert_eq!(stake_info.accumulated_reward, 0);
    }

    #[test]
    fn test_get_stake_info_with_rewards() {
        // Set up the testing environment
        let initial_timestamp = 0;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, initial_timestamp);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate a user staking tokens
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);
        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        // Simulate time passing (1 year)
        let new_timestamp = initial_timestamp + 365 * 24 * 60 * 60 * 1_000_000_000; // Add 1 year in nanoseconds
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, new_timestamp);
        testing_env!(context.build());

        // Get stake info with real-time rewards
        let stake_info = contract.get_stake_info(sender_id).unwrap();

        // Calculate expected rewards
        let expected_rewards = (stake_amount.0
            * ((AAR_EARLY[0] + AAR_EARLY[1] + AAR_EARLY[2] + AAR_EARLY[3] + AAR_EARLY[4])
                * WEEK as u128
                + AAR * (SECONDS_IN_A_YEAR - 5 * WEEK as u128)))
            / (SECONDS_IN_A_YEAR * 10000);

        // Assert that the accumulated reward matches the expected rewards
        assert_eq!(stake_info.accumulated_reward, expected_rewards);
    }

    #[test]
    fn test_stake_and_unstake() {
        // Set up the testing environment
        let initial_timestamp = 0;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, initial_timestamp);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate a user staking tokens
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);
        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        // Simulate time passing (1 year)
        let new_timestamp = initial_timestamp + 7 * 24 * 60 * 60 * 1_000_000_000;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, new_timestamp);
        testing_env!(context.build());

        // Get stake info with real-time rewards
        let stake_info = contract.get_stake_info(sender_id).unwrap();

        // Calculate expected rewards
        let expected_rewards =
            (stake_amount.0 * ((AAR_EARLY[0]) * WEEK as u128)) / (SECONDS_IN_A_YEAR * 10000);

        // Assert that the accumulated reward matches the expected rewards
        assert_eq!(stake_info.accumulated_reward, expected_rewards);
    }

    #[test]
    fn test_stake_and_unstake2() {
        // Set up the testing environment
        let initial_timestamp = 0;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, initial_timestamp);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate time passing (1 year)
        let new_timestamp = initial_timestamp + 5 * 7 * 24 * 60 * 60 * 1_000_000_000;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, new_timestamp);
        testing_env!(context.build());

        // Simulate a user staking tokens
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);
        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        let new_timestamp2 = new_timestamp + 365 * 24 * 60 * 60 * 1_000_000_000;
        let context2 = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, new_timestamp2);
        testing_env!(context2.build());
        // Get stake info with real-time rewards
        let stake_info = contract.get_stake_info(sender_id).unwrap();

        // Calculate expected rewards
        let expected_rewards = (stake_amount.0 * AAR) / 10000;

        // Assert that the accumulated reward matches the expected rewards
        assert_eq!(stake_info.accumulated_reward, expected_rewards);
    }

    #[test]
    fn test_unstaking() {
        // Set up the testing environment
        let initial_timestamp = 0;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, initial_timestamp);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(
            accounts(0),
            TOKEN_CONTRACT.parse().unwrap(),
            U128(1_000_000u128),
        );

        // Simulate a user staking tokens
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);
        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        // Simulate time passing (1 year)
        let new_timestamp = initial_timestamp + 365 * 24 * 60 * 60 * 1_000_000_000; // Add 1 year in nanoseconds
        let context = get_context(accounts(1), 1, new_timestamp);
        testing_env!(context.build());

        let mut stake_info = contract.get_stake_info(sender_id.clone());
        // Unstake all tokens
        contract.unstake();
        let stake = stake_info.unwrap();
        contract.on_ft_transfer_then_remove(
            accounts(1),
            stake.amount,
            stake.accumulated_reward,
            stake.first_stake_time,
            stake.start_time,
            stake.accumulated_reward,
            Ok(()),
        );
        // Check that the user's staking record is removed
        stake_info = contract.get_stake_info(sender_id);
        assert!(stake_info.is_none());
        assert_eq!(contract.get_total_stake(), 0);
        assert_eq!(
            contract.get_total_claimed_reward(),
            stake.accumulated_reward
        );
    }
}
