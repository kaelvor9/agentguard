# AgentGuard vault contract

Rust smart contract (near-sdk 5.29) that holds the funds and enforces the payment policy. See the [main README](../README.md) for the full design.

| Method | Caller | Purpose |
|--------|--------|---------|
| `new(owner, agent, guardian, policy)` | the vault account itself (init) | Set roles and policy |
| `request_payment(to, amount, memo) -> id` | agent | Ask for a payment; hard limits are checked immediately |
| `submit_risk(id, score, reason) -> status` | guardian | Post the risk score; the vault pays, holds or rejects |
| `approve(id)`, `reject(id)` | owner, 1 yoctoNEAR | Resolve a held payment |
| `set_policy`, `add_payee`, `remove_payee`, `pause`, `unpause`, `set_agent`, `set_guardian`, `withdraw` | owner, 1 yoctoNEAR | Administration |
| `get_config`, `get_payees`, `get_request`, `list_requests`, `get_spending` | anyone (view) | Read state |

Amounts are in yoctoNEAR (1 NEAR = 10^24). Every status change is logged as a NEP-297 `EVENT_JSON` event with `standard: "agentguard"`.

```
cargo test                                   # 12 unit tests + 1 sandbox end-to-end test
cargo near build non-reproducible-wasm       # build target/near/contract.wasm
```
