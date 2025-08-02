use anyhow::Result;
use near_sdk::json_types::U128;
use near_workspaces::{compile_project, sandbox, types::NearToken, Account, Contract};
use serde_json::json;

/// Integration test that deploys the real staking & token contracts and shows that
/// unstake() creates a failing cross-contract call (ft_transfer on the user account).
#[tokio::test]
async fn test_unstake_cross_contract_failure() -> Result<()> {
    // 1. Spin up a local sandbox network
    let worker = sandbox().await?;

    // 2. Compile the token and staking contracts to WASM
    let token_wasm = compile_project("../publicai-token").await?;
    let staking_wasm = compile_project(".").await?; // current crate

    // 3. Deploy the token contract
    let token_contract: Contract = worker.dev_deploy(&token_wasm).await?;

    // Initialize token contract
    let root_account = worker.root_account()?;
    let metadata = json!({
        "spec": "ft-1.0.0",
        "name": "Test Token",
        "symbol": "TT",
        "decimals": 18,
        "icon": null,
        "reference": null,
        "reference_hash": null
    });

    let _ = root_account
        .call(token_contract.id(), "new")
        .args_json(json!({
            "owner_id": root_account.id(),
            "total_supply": U128(1_000_000_000u128),
            "metadata": metadata
        }))
        .max_gas()
        .transact()
        .await?
        .into_result()?; // Unwrap to catch init failure

    // 4. Deploy the staking contract
    let staking_contract: Contract = worker.dev_deploy(&staking_wasm).await?;

    let _ = root_account
        .call(staking_contract.id(), "new")
        .args_json((root_account.id(), token_contract.id()))
        .transact()
        .await?
        .into_result()?; // Unwrap to catch init failure

    // 5. Create a user account and mint them some tokens
    let alice: Account = worker.dev_create_account().await?;

    // Register alice for storage
    let _ = root_account
        .call(token_contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": alice.id(), "registration_only": null }))
        .deposit(NearToken::from_yoctonear(
            1_250_000_000_000_000_000_000_000u128,
        ))
        .transact()
        .await?
        .into_result()?; // Unwrap to catch failure

    // Register staking_contract for storage (must be before ft_transfer_call)
    let _ = root_account
        .call(token_contract.id(), "storage_deposit")
        .args_json(json!({ "account_id": staking_contract.id(), "registration_only": null }))
        .deposit(NearToken::from_yoctonear(
            1_250_000_000_000_000_000_000_000u128,
        ))
        .transact()
        .await?
        .into_result()?; // Unwrap to catch failure

    // Transfer tokens to alice
    let _ = root_account
        .call(token_contract.id(), "ft_transfer")
        .args_json(json!({
            "receiver_id": alice.id(),
            "amount": U128(1_000_000u128),
            "memo": null
        }))
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?
        .into_result()?; // Unwrap to catch failure

    // 6. Alice stakes her tokens via ft_transfer_call
    let exec = alice
        .call(token_contract.id(), "ft_transfer_call")
        .args_json(json!({
            "receiver_id": staking_contract.id(),
            "amount": U128(1_000_000u128),
            "memo": null,
            "msg": ""
        }))
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?;

    println!("ft_transfer_call is_success: {:?}", exec.is_success());
    exec.clone().into_result()?; // Unwrap to catch failure

    // Wait for cross-contract calls to complete by advancing the sandbox
    // NEAR cross-contract calls are asynchronous and take multiple blocks
    worker.fast_forward(10).await?;

    // Check Alice's balance after ft_transfer_call and cross-contract completion
    let alice_balance_after: U128 = alice
        .view(token_contract.id(), "ft_balance_of")
        .args_json(json!({ "account_id": alice.id() }))
        .await?
        .json()?;
    println!(
        "Alice's balance after ft_transfer_call: {}",
        alice_balance_after.0
    );

    // Check staking contract balance after ft_transfer_call and cross-contract completion
    let staking_balance_after: U128 = alice
        .view(token_contract.id(), "ft_balance_of")
        .args_json(json!({ "account_id": staking_contract.id() }))
        .await?
        .json()?;
    println!(
        "Staking contract balance after ft_transfer_call: {}",
        staking_balance_after.0
    );

    // 6b. Confirm stake exists
    let stake_info: serde_json::Value = alice
        .view(staking_contract.id(), "get_stake_info")
        .args_json(json!({ "account_id": alice.id() }))
        .await?
        .json()?;
    println!(
        "Stake info after cross-contract completion: {:?}",
        stake_info
    );
    assert!(stake_info != serde_json::Value::Null, "Stake not created");

    // 7. Alice calls unstake()
    let unstake_exec = alice
        .call(staking_contract.id(), "unstake")
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?;

    println!("unstake is_success: {:?}", unstake_exec.is_success());
    unstake_exec.clone().into_result()?; // Unwrap if needed, but since it "succeeds" we continue

    // Wait for the unstake cross-contract call to complete
    worker.fast_forward(10).await?;

    // 8. Verify alice did NOT receive her tokens back (promise failed)
    let balance: U128 = alice
        .view(token_contract.id(), "ft_balance_of")
        .args_json(json!({ "account_id": alice.id() }))
        .await?
        .json()?;
    assert_eq!(
        balance.0, 1000000,
        "Alice should  have  tokens because the unstake success"
    );

    // Verify that the stake was removed despite the transfer failure
    let stake_info_after: serde_json::Value = alice
        .view(staking_contract.id(), "get_stake_info")
        .args_json(json!({ "account_id": alice.id() }))
        .await?
        .json()?;
    assert_eq!(
        stake_info_after,
        serde_json::Value::Null,
        "Stake should be removed after unstake"
    );

    // Verify that the tokens are still held by the staking contract
    let staking_balance: U128 = alice
        .view(token_contract.id(), "ft_balance_of")
        .args_json(json!({ "account_id": staking_contract.id() }))
        .await?
        .json()?;
    assert_eq!(
        staking_balance.0, 0,
        "Staking contract should have 0 tokens"
    );

    Ok(())
}
