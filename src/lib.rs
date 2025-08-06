use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::collections::UnorderedMap;
use near_sdk::json_types::U128;
use near_sdk::{
    assert_one_yocto, env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue,
};

// Constants
const APY: u128 = 800; // Annual Percentage Yield (8%)
const SECONDS_IN_A_YEAR: u128 = 365 * 24 * 60 * 60; // Number of seconds in a year
const NANOSECONDS: u64 = 1_000_000_000; // Nanoseconds to seconds
const APY_BASE: u128 = 10000;

/// Struct for storing staking information
#[near(serializers = [json, borsh])]
pub struct StakeInfo {
    amount: u128,             // The principal amount staked by the user
    accumulated_reward: u128, // Accumulated interest rewards
    start_time: u64,          // Timestamp when staking began
}

/// Main contract struct
#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct StakingContract {
    owner_id: AccountId,                                 // Contract owner
    token_contract: AccountId,                           // NEP-141 token contract address
    staked_balances: UnorderedMap<AccountId, StakeInfo>, // User staking information
    stake_paused: bool,                                  // Pause stake
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
        Self {
            owner_id,
            token_contract,
            staked_balances: UnorderedMap::new(b"s".to_vec()),
            stake_paused: false,
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

        // Calculate the time difference and accumulated rewards
        let current_time = env::block_timestamp() / NANOSECONDS; // Convert nanoseconds to seconds
        let reward_end_time = if self.stake_end_time == 0 {
            current_time
        } else {
            std::cmp::min(current_time, self.stake_end_time)
        };

        let staked_duration = if reward_end_time >= stake_info.start_time {
            reward_end_time - stake_info.start_time
        } else {
            0
        };

        // Update accumulated rewards
        let reward = self.calculate_reward(stake_info.amount, staked_duration);
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
        stake_info.accumulated_reward += claim_reward;

        // Total payout = principal + accumulated rewards
        let total_payout = stake_info.amount + stake_info.accumulated_reward;

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
                        stake_info.accumulated_reward,
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
        #[callback_result] call_result: Result<(), near_sdk::PromiseError>,
    ) -> bool {
        assert!(call_result.is_ok(), "Unstake transfer failed");
        // Remove staking record
        self.staked_balances.remove(&account_id);
        self.total_staked -= stake_amount;
        self.total_claimed_reward += reward_amount;
        true
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

            let staked_duration = if reward_end_time >= stake_info.start_time {
                reward_end_time - stake_info.start_time
            } else {
                0
            };

            // Calculate real-time rewards
            let reward = self.calculate_reward(stake_info.amount, staked_duration);

            // Update the accumulated reward (real-time)
            stake_info.accumulated_reward += reward;

            // Return the updated stake info with real-time rewards
            Some(stake_info)
        } else {
            None
        }
    }

    /// Calculate rewards based on staking amount and duration
    fn calculate_reward(&self, amount: u128, duration_in_seconds: u64) -> u128 {
        // Reward formula: Principal * APY * duration / (SECONDS_IN_A_YEAR * 10000)
        let reward = amount * APY * (duration_in_seconds as u128) / (SECONDS_IN_A_YEAR * APY_BASE);
        reward
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

        // Get the current timestamp
        let current_time = env::block_timestamp() / NANOSECONDS; // Convert nanoseconds to seconds

        // Update or create the user's staking record
        let mut stake_info = self.staked_balances.get(&sender_id).unwrap_or(StakeInfo {
            amount: 0,
            accumulated_reward: 0,
            start_time: current_time,
        });

        // Update accumulated rewards
        let staked_duration = current_time - stake_info.start_time;
        let reward = self.calculate_reward(stake_info.amount, staked_duration);
        stake_info.accumulated_reward += reward;

        // Update principal and timestamp
        stake_info.amount += amount.0;
        stake_info.start_time = current_time;

        self.staked_balances.insert(&sender_id, &stake_info);

        self.total_staked += amount.0;
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
        let expected_rewards = (stake_amount.0 * APY) / 10000;

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
