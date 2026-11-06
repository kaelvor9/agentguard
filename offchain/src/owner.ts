// Owner actions from the terminal, signed with the owner's full-access key.
//   npm run owner -- approve <id> | reject <id> | pause | unpause
import { call, config, loadAccount } from "./near.js";

const [action, id] = process.argv.slice(2);
const ONE_YOCTO = 1n; // forces a full-access key: function-call keys can't attach deposits

const methods: Record<string, () => [string, Record<string, unknown>]> = {
  approve: () => ["approve", { id }],
  reject: () => ["reject", { id }],
  pause: () => ["pause", {}],
  unpause: () => ["unpause", {}],
};

if (!action || !methods[action] || ((action === "approve" || action === "reject") && !id)) {
  console.log("Usage: npm run owner -- approve <id> | reject <id> | pause | unpause");
  process.exit(1);
}

const [method, args] = methods[action]();
const owner = loadAccount(config.owner);
const { result, txHash } = await call<unknown>(owner, method, args, ONE_YOCTO);
console.log(`${method} ok${result !== undefined ? `: ${JSON.stringify(result)}` : ""}   ${config.explorer}/${txHash}`);
