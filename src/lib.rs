use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{env, near_bindgen, AccountId, PromiseOrValue, PanicOnDefault, NearToken, near};
use near_sdk::collections::UnorderedMap;
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::json_types::U128;

// Constants
const APY: u128 = 250; // Annual Percentage Yield (2.5%)
const SECONDS_IN_A_YEAR: u128 = 365 * 24 * 60 * 60; // Number of seconds in a year

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
    owner_id: AccountId,                                // Contract owner
    token_contract: AccountId,                         // NEP-141 token contract address
    staked_balances: UnorderedMap<AccountId, StakeInfo>, // User staking information
}

#[near]
impl StakingContract {
    /// Initialize the contract
    #[init]
    pub fn new(owner_id: AccountId, token_contract: AccountId) -> Self {
        assert!(!env::state_exists(), "Already initialized");
        Self {
            owner_id,
            token_contract,
            staked_balances: UnorderedMap::new(b"s".to_vec()),
        }
    }

    /// Unstake all principal and rewards
    #[payable]
    pub fn unstake(&mut self) {
        let account_id = env::predecessor_account_id();
        let mut stake_info = self
            .staked_balances
            .get(&account_id)
            .expect("No stake found for this account");

        // Calculate the time difference and accumulated rewards
        let current_time = env::block_timestamp() / 1_000_000_000; // Convert nanoseconds to seconds
        let staked_duration = current_time - stake_info.start_time;

        // Update accumulated rewards
        let reward = self.calculate_reward(stake_info.amount, staked_duration);
        stake_info.accumulated_reward += reward;

        // Total payout = principal + accumulated rewards
        let total_payout = stake_info.amount + stake_info.accumulated_reward;

        // Remove staking record
        self.staked_balances.remove(&account_id);

        // Transfer principal and rewards to the user
        near_sdk::Promise::new(account_id.clone()).function_call(
            "ft_transfer".to_string(),
            near_sdk::serde_json::json!({
                "receiver_id": account_id,
                "amount": total_payout.to_string(),
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1), // Attach 1 yoctoNEAR
            env::prepaid_gas().saturating_div(2),
        );
    }

    /// Query staking information for a specific user
    pub fn get_stake_info(&self, account_id: AccountId) -> Option<StakeInfo> {
        if let Some(mut stake_info) = self.staked_balances.get(&account_id) {
            // Calculate the time difference
            let current_time = env::block_timestamp() / 1_000_000_000; // Convert nanoseconds to seconds
            let staked_duration = current_time - stake_info.start_time;

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
        let reward = amount * APY * (duration_in_seconds as u128) / (SECONDS_IN_A_YEAR * 10000);
        reward
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
        _msg: String,
    ) -> PromiseOrValue<U128> {
        // Ensure that the token being transferred is the one specified in the contract
        assert_eq!(
            env::predecessor_account_id(),
            self.token_contract,
            "Only the specified token can be staked"
        );

        // Get the current timestamp
        let current_time = env::block_timestamp() / 1_000_000_000; // Convert nanoseconds to seconds

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

        // Return 0 to indicate the transfer was successfully handled
        PromiseOrValue::Value(U128(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{AccountId, testing_env, test_utils::VMContextBuilder, MockedBlockchain};
    use near_sdk::test_utils::accounts;
    use near_sdk::json_types::U128;

    const TOKEN_CONTRACT: &str = "token.testnet";

    /// Helper function to create a mock context
    fn get_context(predecessor: AccountId, attached_deposit: u128, block_timestamp: u64) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .predecessor_account_id(predecessor) // The account that sends the call (e.g., the token contract)
            .attached_deposit(NearToken::from_yoctonear(attached_deposit)) // The deposit attached with the call
            .block_timestamp(block_timestamp);  // Set the block timestamp
        builder
    }

    #[test]
    fn test_contract_initialization() {
        // Set up the testing environment
        let context = get_context(accounts(0), 0, 0);
        testing_env!(context.build());

        // Initialize the contract
        let token_contract: AccountId = TOKEN_CONTRACT.parse().unwrap();
        let contract = StakingContract::new(accounts(0), token_contract.clone());

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
        let mut contract = StakingContract::new(accounts(0), TOKEN_CONTRACT.parse().unwrap());

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
        let mut contract = StakingContract::new(accounts(0), TOKEN_CONTRACT.parse().unwrap());

        // Simulate a user staking tokens multiple times
        let sender_id = accounts(1);
        let first_stake_amount = U128(1_000_000);
        let second_stake_amount = U128(500_000);

        contract.ft_on_transfer(sender_id.clone(), first_stake_amount, "".to_string());
        contract.ft_on_transfer(sender_id.clone(), second_stake_amount, "".to_string());

        // Check if the user's staking record is updated
        let stake_info = contract.get_stake_info(sender_id).unwrap();
        assert_eq!(stake_info.amount, first_stake_amount.0 + second_stake_amount.0);
        assert_eq!(stake_info.accumulated_reward, 0);
    }

    #[test]
    fn test_get_stake_info_with_rewards() {
        // Set up the testing environment
        let initial_timestamp = 0;
        let context = get_context(TOKEN_CONTRACT.parse().unwrap(), 0, initial_timestamp);
        testing_env!(context.build());

        // Initialize the contract
        let mut contract = StakingContract::new(accounts(0), TOKEN_CONTRACT.parse().unwrap());

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
        let mut contract = StakingContract::new(accounts(0), TOKEN_CONTRACT.parse().unwrap());

        // Simulate a user staking tokens
        let sender_id = accounts(1);
        let stake_amount = U128(1_000_000);
        contract.ft_on_transfer(sender_id.clone(), stake_amount, "".to_string());

        // Simulate time passing (1 year)
        let new_timestamp = initial_timestamp + 365 * 24 * 60 * 60 * 1_000_000_000; // Add 1 year in nanoseconds
        let context = get_context(accounts(1), 0, new_timestamp);
        testing_env!(context.build());

        // Unstake all tokens
        contract.unstake();

        // Check that the user's staking record is removed
        let stake_info = contract.get_stake_info(sender_id);
        assert!(stake_info.is_none());
    }
}
