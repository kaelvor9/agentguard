// Prints the vault's policy, today's spending and recent payment requests.
import { config, type PaymentRequest, type Policy, toNear, view } from "./near.js";

const [cfg, spending, requests] = await Promise.all([
  view<{ owner: string; agent: string; guardian: string; paused: boolean; policy: Policy }>("get_config"),
  view<{ spent_today: string; remaining_today: string; available_balance: string }>("get_spending"),
  view<PaymentRequest[]>("list_requests", { limit: 15 }),
]);

const p = cfg.policy;
console.log(`Vault ${config.vault}${cfg.paused ? "   ** PAUSED **" : ""}`);
console.log(`  owner ${cfg.owner} | agent ${cfg.agent} | guardian ${cfg.guardian}`);
console.log(
  `  policy: max ${toNear(p.per_tx_limit)}/payment, ${toNear(p.daily_limit)}/day, approval above ${toNear(p.approval_threshold)}, hold at risk ${p.risk_hold}, block at ${p.risk_block}`,
);
console.log(
  `  today: spent ${toNear(spending.spent_today)}, remaining ${toNear(spending.remaining_today)} | vault balance ${toNear(spending.available_balance)} NEAR\n`,
);

for (const r of requests) {
  const risk = r.risk_score === null ? "  -" : String(r.risk_score).padStart(3);
  console.log(
    `#${r.id.padEnd(3)} ${r.status.padEnd(13)} risk ${risk}  ${toNear(r.amount).padStart(8)} NEAR -> ${r.to.padEnd(28)} ${r.decision ?? ""}`,
  );
  if (r.risk_reason) console.log(`      guardian: ${r.risk_reason}`);
}
if (requests.length === 0) console.log("No payment requests yet.");
