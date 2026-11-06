// Pure logic shared by the agent and guardian (no network), so it can be unit-tested.
import type { PaymentRequest, Policy } from "./near.js";

export interface Payment {
  to: string;
  amount_near: number;
}

export interface Score {
  score: number;
  reason: string;
}

const ACCOUNT = /([a-z0-9][a-z0-9._-]*\.(?:testnet|near))/;

/** Rule-based invoice reader. Like a naive agent, the last payment instruction in the text wins. */
export function parseInvoice(text: string): Payment | null {
  let to = text.match(new RegExp(`pay to:\\s*${ACCOUNT.source}`, "i"))?.[1];
  let amount = text.match(/amount:\s*([\d.]+)\s*NEAR/i)?.[1];
  const instructions = [
    ...text.matchAll(new RegExp(`(?:send|pay|transfer)\\s+([\\d.]+)\\s*NEAR\\s+to\\s+${ACCOUNT.source}`, "gi")),
  ];
  const last = instructions.at(-1);
  if (last) {
    amount = last[1];
    to = last[2];
  }
  return to && amount ? { to, amount_near: Number(amount) } : null;
}

const INJECTION = /ignore (all |any )?(previous|prior|above)|disregard|override|system (note|prompt|message)|you are now|new instructions/i;
const PRESSURE = /urgent|immediately|final notice|within \d+ (minutes|hours)|act now|suspended|last chance/i;
const SCAM = /verify (your )?wallet|refund desk|refund-desk|recover(y)? (fee|funds)|seed phrase|private key|gift card|wallet (check|validation)/i;

/** Red-flag rules. Each flag adds points; capped at 100. */
export function ruleScore(
  req: Pick<PaymentRequest, "to" | "amount" | "memo">,
  payees: Map<string, string>,
  history: Pick<PaymentRequest, "to" | "status">[],
  policy: Pick<Policy, "approval_threshold">,
): Score {
  let score = 0;
  const flags: string[] = [];
  const add = (points: number, flag: string) => {
    score += points;
    flags.push(flag);
  };
  const paidBefore = history.some((h) => h.to === req.to && h.status === "Executed");

  if (!payees.has(req.to)) add(35, "payee not on allowlist");
  if (!paidBefore) add(10, "first payment to this payee");
  if (INJECTION.test(req.memo)) add(60, "prompt-injection text in memo");
  if (PRESSURE.test(req.memo)) add(25, "pressure/urgency language");
  if (SCAM.test(req.memo)) add(25, "known scam pattern");
  if (BigInt(req.amount) > BigInt(policy.approval_threshold)) add(10, "large amount");

  return {
    score: Math.min(score, 100),
    reason: flags.length ? flags.join("; ") : "known payee, normal amount, no red flags",
  };
}
