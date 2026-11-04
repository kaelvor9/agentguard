//! End-to-end test on a local NEAR sandbox: request -> risk score -> payment.
use near_api::{AccountId, NearGas, NearToken};
use near_sdk::serde_json::{Value, json};

async fn full_flow(contract_wasm: Vec<u8>) -> testresult::TestResult<()> {
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let net = near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let signer = near_api::Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse().unwrap(),
    )?;

    let owner = create_account(&sandbox, "owner.sandbox", 10).await?;
    let agent = create_account(&sandbox, "agent.sandbox", 10).await?;
    let guardian = create_account(&sandbox, "guardian.sandbox", 10).await?;
    let shop = create_account(&sandbox, "shop.sandbox", 10).await?;
    let vault = create_account(&sandbox, "vault.sandbox", 50).await?.as_contract();

    // Deploy + init in one transaction
    near_api::Contract::deploy(vault.account_id().clone())
        .use_code(contract_wasm)
        .with_init_call(
            "new",
            json!({
                "owner": owner.account_id(),
                "agent": agent.account_id(),
                "guardian": guardian.account_id(),
                "policy": {
                    "per_tx_limit": NearToken::from_near(5),
                    "daily_limit": NearToken::from_near(10),
                    "approval_threshold": NearToken::from_near(3),
                    "risk_hold": 60,
                    "risk_block": 90
                }
            }),
        )?
        .with_signer(signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    // Owner allowlists the shop (1 yoctoNEAR attached)
    vault
        .call_function("add_payee", json!({"account": shop.account_id(), "label": "API credits"}))
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    let shop_before = shop.tokens().near_balance().fetch_from(&net).await?.total;

    // Agent requests 1 NEAR
    vault
        .call_function(
            "request_payment",
            json!({"to": shop.account_id(), "amount": NearToken::from_near(1), "memo": "invoice 1"}),
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    // The agent cannot score its own request
    vault
        .call_function("submit_risk", json!({"id": "0", "score": 0, "reason": "trust me"}))
        .transaction()
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_failure();

    // Guardian scores it low -> vault pays
    vault
        .call_function("submit_risk", json!({"id": "0", "score": 10, "reason": "known payee"}))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(guardian.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    let request: Value = vault
        .call_function("get_request", json!({"id": "0"}))
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    assert_eq!(request["status"], "Executed");

    let shop_after = shop.tokens().near_balance().fetch_from(&net).await?.total;
    assert_eq!(shop_after, shop_before.saturating_add(NearToken::from_near(1)));

    // A 50 NEAR request is rejected on the spot
    vault
        .call_function(
            "request_payment",
            json!({"to": shop.account_id(), "amount": NearToken::from_near(50), "memo": "URGENT"}),
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();
    let request: Value = vault
        .call_function("get_request", json!({"id": "1"}))
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    assert_eq!(request["status"], "Rejected");

    Ok(())
}

async fn create_account(
    sandbox: &near_sandbox::Sandbox,
    name: &str,
    near: u128,
) -> testresult::TestResult<near_api::Account> {
    let account_id: AccountId = name.parse().unwrap();
    sandbox
        .create_account(account_id.clone())
        .initial_balance(NearToken::from_near(near))
        .send()
        .await?;
    Ok(near_api::Account(account_id))
}

#[tokio::test]
async fn test_full_payment_flow() -> testresult::TestResult<()> {
    let contract_wasm_path = cargo_near_build::build_with_cli(Default::default())?;
    let contract_wasm = std::fs::read(contract_wasm_path)?;
    full_flow(contract_wasm).await
}
