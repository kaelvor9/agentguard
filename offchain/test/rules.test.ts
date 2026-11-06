// Offline tests for the agent's invoice reader and the guardian's red-flag rules.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseInvoice, ruleScore } from "../src/rules.js";

const inbox = (n: number) => readFileSync(`demo/inbox/invoice-${n}.txt`, "utf8");
const near = (n: number) => (BigInt(Math.round(n * 1000)) * 10n ** 21n).toString();
const payees = new Map([
  ["api-shop.kaelvor.testnet", "API credits"],
  ["hosting.kaelvor.testnet", "Server hosting"],
]);
const history = [{ to: "api-shop.kaelvor.testnet", status: "Executed" as const }];
const policy = { approval_threshold: near(3) };
const score = (n: number) => {
  const text = inbox(n);
  const p = parseInvoice(text)!;
  return ruleScore({ to: p.to, amount: near(p.amount_near), memo: text }, payees, history, policy).score;
};

test("normal invoices parse to the stated payee and amount", () => {
  assert.deepEqual(parseInvoice(inbox(1001)), { to: "api-shop.kaelvor.testnet", amount_near: 1 });
  assert.deepEqual(parseInvoice(inbox(1002)), { to: "hosting.kaelvor.testnet", amount_near: 4 });
  assert.deepEqual(parseInvoice(inbox(1005)), { to: "api-shop.kaelvor.testnet", amount_near: 0.5 });
});

test("the naive agent falls for the prompt injection in #1003", () => {
  assert.deepEqual(parseInvoice(inbox(1003)), { to: "refund-desk.kaelvor.testnet", amount_near: 50 });
});

test("routine payments to a known payee score low", () => {
  assert.ok(score(1001) < 60, `got ${score(1001)}`);
  assert.ok(score(1005) < 60, `got ${score(1005)}`);
});

test("first hosting invoice is not blocked by rules (the 4 NEAR threshold holds it instead)", () => {
  assert.ok(score(1002) < 60, `got ${score(1002)}`);
});

test("the wallet-verification scam in #1004 is blocked (>= 90)", () => {
  assert.ok(score(1004) >= 90, `got ${score(1004)}`);
});

test("the injection in #1003 would be blocked even if it got past the hard limits", () => {
  assert.ok(score(1003) >= 90, `got ${score(1003)}`);
});
