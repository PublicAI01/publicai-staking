

---

# Staking Contract

This is a NEAR staking smart contract that allows users to stake NEP-141 tokens and earn rewards based on the Annual Percentage Yield (APY). Users can stake tokens multiple times, query their current stake and rewards, and unstake to retrieve their principal and accumulated rewards.

---

## Features

1. **Stake NEP-141 Tokens**: Users can stake tokens by transferring them to the contract using the `ft_transfer_call` method.
2. **Multi-Stake Support**: Users can stake multiple times, and their total stake will be accumulated.
3. **Real-Time Rewards Calculation**: The contract dynamically calculates rewards based on the staking duration and APY.
4. **Unstake**: Users can unstake their tokens at any time to retrieve their principal and accumulated rewards.
5. **Query Staking Information**: Users can query their staking details, including the current principal and real-time rewards.

---

## Contract Details

### Constants

- **APY**: `2.5%` (represented as `250` with a precision factor of `10000`).
- **SECONDS_IN_A_YEAR**: `31,536,000` seconds (365 days).

---

### Methods

#### Initialization

```rust
pub fn new(owner_id: AccountId, token_contract: AccountId) -> Self
```

Initializes the contract with the following parameters:
- `owner_id`: The account ID of the contract owner.
- `token_contract`: The NEP-141 token contract address to be used for staking.

---

#### Staking (NEP-141 `ft_transfer_call`)

```rust
fn ft_on_transfer(
    &mut self,
    sender_id: AccountId,
    amount: U128,
    _msg: String,
) -> PromiseOrValue<U128>
```

Automatically called when a user stakes tokens using the `ft_transfer_call` method of the NEP-141 token contract. It updates the user's staking record, adding the new stake to the existing balance and recalculating rewards.

---

#### Query Staking Information

```rust
pub fn get_stake_info(&self, account_id: AccountId) -> Option<StakeInfo>
```

Returns the staking details for the given `account_id`, including:
- `amount`: The total principal staked by the user.
- `accumulated_reward`: The rewards earned so far, including real-time calculations.
- `start_time`: The timestamp when staking began.

---

#### Unstake

```rust
pub fn unstake(&mut self)
```

Allows users to retrieve their entire principal and accumulated rewards. After unstaking, the user's staking record is removed from the contract.

---

#### Internal Methods

##### Calculate Rewards

```rust
fn calculate_reward(&self, amount: u128, duration_in_seconds: u64) -> u128
```

Calculates the rewards based on the staking amount, staking duration, APY, and the total seconds in a year using the following formula:

```
Reward = Principal * APY * Duration / (SECONDS_IN_A_YEAR * 10000)
```

---

## Usage

### Deploying the Contract

1. Compile the contract:
   ```bash
   cargo build --target wasm32-unknown-unknown --release
   ```

2. Deploy the contract to a NEAR account:
   ```bash
   near deploy --accountId <contract_account_id> --wasmFile <path_to_wasm_file>
   ```

3. Initialize the contract:
   ```bash
   near call <contract_account_id> new '{"owner_id": "<owner_account_id>", "token_contract": "<token_contract_id>"}' --accountId <owner_account_id>
   ```

---

### Staking Tokens

Users can stake NEP-141 tokens by calling the `ft_transfer_call` method on the token contract.

Example command:
```bash
near call <token_contract_id> ft_transfer_call '{"receiver_id": "<contract_account_id>", "amount": "1000000000000000000000000", "msg": ""}' --accountId <user_account_id> --depositYocto 1
```

---

### Query Staking Information

Users can query their staking details, including real-time rewards, using the `get_stake_info` method.

Example command:
```bash
near view <contract_account_id> get_stake_info '{"account_id": "<user_account_id>"}'
```

---

### Unstaking Tokens

Users can unstake their tokens to retrieve their principal and accumulated rewards by calling the `unstake` method.

Example command:
```bash
near call <contract_account_id> unstake '{}' --accountId <user_account_id> --depositYocto 1
```

After unstaking, the user's staking record is removed from the contract.

---

## Testing

To test this contract, you can use NEAR SDK's simulation framework. The `tests/staking_contract.rs` file contains unit tests for the following functionalities:
1. Contract initialization.
2. Staking tokens using `ft_on_transfer`.
3. Querying staking details with real-time rewards.
4. Unstaking tokens and removing staking records.

Run the tests with the following command:
```bash
cargo test -- --nocapture
```

---

## Example Test Cases

### Contract Initialization

Ensures the contract correctly initializes with the provided owner and token contract.

### Staking Tokens

Tests that staking tokens updates the user's staking record with the correct principal and rewards.

### Multiple Staking

Verifies that multiple staking operations accumulate the user's principal and rewards correctly.

### Querying Staking Information

Checks that `get_stake_info` returns accurate real-time rewards.

### Unstaking Tokens

Tests that unstaking retrieves the correct principal and rewards, and removes the user's staking record.

---

## Notes

- Ensure the contract account has enough NEAR to cover storage costs.
- The contract uses integer calculations to avoid floating-point errors in reward calculations.
- Rewards are calculated dynamically based on real-time staking duration.

---

## License

This contract is open-source and available under the MIT License.

---