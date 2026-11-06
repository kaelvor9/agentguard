// Shared NEAR helpers: config, keys, view calls and signed calls.
import "dotenv/config";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  Account,
  FailoverRpcProvider,
  JsonRpcProvider,
  KeyPairSigner,
  type KeyPairString,
  type Provider,
  parseNearAmount,
  yoctoToNear,
} from "near-api-js";

export const config = {
  vault: process.env.VAULT ?? "vault.kaelvor.testnet",
  agent: process.env.AGENT ?? "agent.kaelvor.testnet",
  guardian: process.env.GUARDIAN ?? "guardian.kaelvor.testnet",
  owner: process.env.OWNER ?? "kaelvor.testnet",
  // Several free endpoints: if one rate-limits us, the next one is tried.
  rpcUrls: (
    process.env.RPC_URLS ??
    "https://test.rpc.fastnear.com,https://rpc.testnet.near.org,https://archival-rpc.testnet.fastnear.com"
  )
    .split(",")
    .map((u) => u.trim())
    .filter(Boolean),
  explorer: "https://testnet.nearblocks.io/txns",
};

export const provider = new FailoverRpcProvider(config.rpcUrls.map((url) => new JsonRpcProvider({ url })));

export type Status = "PendingRisk" | "NeedsApproval" | "Executed" | "Rejected" | "Expired";

export interface PaymentRequest {
  id: string;
  to: string;
  amount: string; // yoctoNEAR
  memo: string;
  status: Status;
  risk_score: number | null;
  risk_reason: string | null;
  decision: string | null;
  created_at: string;
}

export interface Policy {
  per_tx_limit: string;
  daily_limit: string;
  approval_threshold: string;
  risk_hold: number;
  risk_block: number;
}

/**
 * Loads an account's key from the near CLI legacy keychain
 * (~/.near-credentials/testnet/<account>.json). The key never appears in code or logs.
 */
export function loadAccount(accountId: string, rpc: Provider = provider): Account {
  const file = join(homedir(), ".near-credentials", "testnet", `${accountId}.json`);
  let privateKey: KeyPairString;
  try {
    privateKey = JSON.parse(readFileSync(file, "utf8")).private_key;
  } catch {
    throw new Error(`No key file for ${accountId} at ${file}`);
  }
  return new Account(accountId, rpc, KeyPairSigner.fromSecretKey(privateKey));
}

/** Read-only call to the vault (free, no signature). */
export async function view<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  return (await provider.callFunction<any>({ contractId: config.vault, method, args })) as T;
}

/** Signed call to the vault. `deposit` is in yoctoNEAR. Returns the transaction hash too. */
export async function call<T>(
  account: Account,
  methodName: string,
  args: Record<string, unknown>,
  deposit: bigint = 0n,
): Promise<{ result: T; txHash: string }> {
  const outcome = await account.callFunctionRaw({
    contractId: config.vault,
    methodName,
    args,
    deposit,
  });
  const status: any = outcome.status;
  if (status?.Failure) {
    throw new Error(`Transaction failed: ${JSON.stringify(status.Failure)}`);
  }
  const raw = status?.SuccessValue ? Buffer.from(status.SuccessValue, "base64").toString() : "";
  const result = (raw ? JSON.parse(raw) : undefined) as T;
  return { result, txHash: outcome.transaction.hash };
}

export const toYocto = (near: number) => parseNearAmount(near);
export const toNear = (yocto: string | bigint) => yoctoToNear(BigInt(yocto), 4);
