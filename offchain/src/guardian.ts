// Guardian: watches the vault for new payment requests, scores each one, posts the score on-chain.
// Final score = max(rule-based score, AI score). Either one alone can stop a bad payment.
import { aiScore } from "./ai-score.js";
import { aiEnabled, model } from "./llm.js";
import { call, config, loadAccount, type PaymentRequest, type Policy, toNear, view } from "./near.js";
import { ruleScore } from "./rules.js";

const POLL_MS = 15_000; // free RPCs rate-limit; 15s keeps us well under
const BACKOFF_MS = 60_000;
const once = process.argv.includes("--once");

async function scorePending(): Promise<void> {
  // One cheap read per poll; payees and policy are only fetched when there is work to do.
  const requests = await view<PaymentRequest[]>("list_requests", { limit: 100 });
  const pending = requests.filter((r) => r.status === "PendingRisk").reverse(); // oldest first
  if (pending.length === 0) return;
  const payees = new Map(await view<[string, string][]>("get_payees"));
  const cfg = await view<{ policy: Policy }>("get_config");

  for (const req of pending) {
    console.log(`\n#${req.id}  ${toNear(req.amount)} NEAR -> ${req.to}`);
    console.log(`  memo: ${req.memo.slice(0, 120)}${req.memo.length > 120 ? "..." : ""}`);

    const rules = ruleScore(req, payees, requests, cfg.policy);
    const ai = await aiScore(req, payees, requests);
    console.log(`  rules: ${rules.score} (${rules.reason})`);
    if (ai) console.log(`  ai:    ${ai.score} (${ai.reason})  [${model}, ${ai.chatId}]`);
    else if (aiEnabled) console.log("  ai:    unavailable for this request, using rules only");

    const final = ai && ai.score > rules.score ? ai : rules;
    // The higher score wins: a confused or manipulated model can never lower the risk the rules found.
    const reason = ai ? `${final.reason} [rules ${rules.score}, ai ${ai.score} via NEAR AI]` : `${final.reason} [rules]`;

    const guardian = loadAccount(config.guardian);
    const { result, txHash } = await call<string>(guardian, "submit_risk", {
      id: req.id,
      score: final.score,
      reason: reason.slice(0, 280),
    });
    console.log(`  => score ${final.score}: vault says ${result}   ${config.explorer}/${txHash}`);
  }
}

async function main() {
  console.log(`Guardian watching ${config.vault} as ${config.guardian}`);
  console.log(aiEnabled ? `AI scoring: NEAR AI Cloud (${model})` : "AI scoring: off (no NEAR_AI_API_KEY), rules only");
  do {
    let wait = POLL_MS;
    try {
      await scorePending();
    } catch (err) {
      const msg = (err as Error).message;
      const limited = /429|rate limit/i.test(msg);
      console.error(limited ? "  RPC rate limit hit, pausing 60s" : `  error: ${msg.slice(0, 200)}`);
      if (limited) wait = BACKOFF_MS;
    }
    if (!once) await new Promise((r) => setTimeout(r, wait));
  } while (!once);
}

main();
