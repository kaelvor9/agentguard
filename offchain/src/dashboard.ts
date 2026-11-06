// Owner console: a local web dashboard for the vault.
//   npm run dashboard   then open http://localhost:3000
// Reads the vault over RPC and signs owner actions (approve, reject, pause, unpause)
// with the owner's key from the near CLI keychain. Listens on 127.0.0.1 only.
import { readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { call, config, loadAccount, type PaymentRequest, type Policy, view } from "./near.js";

const PORT = Number(process.env.DASHBOARD_PORT ?? 3000);
const CACHE_MS = 8_000; // free RPCs rate-limit: share one read across all browser refreshes
const html = () => readFileSync(new URL("../dashboard/index.html", import.meta.url), "utf8");

let cache: { at: number; data: unknown } | null = null;

async function state() {
  if (cache && Date.now() - cache.at < CACHE_MS) return cache.data;
  const cfg = await view<{ owner: string; agent: string; guardian: string; paused: boolean; policy: Policy }>("get_config");
  const spending = await view<{ spent_today: string; remaining_today: string; available_balance: string }>("get_spending");
  const requests = await view<PaymentRequest[]>("list_requests", { limit: 25 });
  const payees = await view<[string, string][]>("get_payees");
  const data = { vault: config.vault, config: cfg, spending, requests, payees };
  cache = { at: Date.now(), data };
  return data;
}

const ACTIONS: Record<string, (id?: string) => [string, Record<string, unknown>]> = {
  approve: (id) => ["approve", { id }],
  reject: (id) => ["reject", { id }],
  pause: () => ["pause", {}],
  unpause: () => ["unpause", {}],
};

function send(res: ServerResponse, status: number, body: unknown, type = "application/json") {
  res.writeHead(status, { "content-type": type, "cache-control": "no-store" });
  res.end(type === "application/json" ? JSON.stringify(body) : String(body));
}

async function readJson(req: IncomingMessage): Promise<Record<string, string>> {
  let raw = "";
  for await (const chunk of req) {
    raw += chunk;
    if (raw.length > 1_000) throw new Error("body too large");
  }
  return raw ? JSON.parse(raw) : {};
}

/**
 * Blocks cross-site requests: without this, any website open in your browser could POST
 * to localhost and approve payments. Browsers can't add the custom header cross-origin
 * without a CORS preflight, which this server never grants.
 */
function sameOrigin(req: IncomingMessage): boolean {
  const origin = req.headers.origin;
  const okOrigin = !origin || origin === `http://localhost:${PORT}` || origin === `http://127.0.0.1:${PORT}`;
  return okOrigin && req.headers["x-agentguard"] === "1";
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? "/", `http://localhost:${PORT}`);
    if (req.method === "GET" && url.pathname === "/") return send(res, 200, html(), "text/html; charset=utf-8");
    if (req.method === "GET" && url.pathname === "/api/state") return send(res, 200, await state());

    const action = url.pathname.match(/^\/api\/(approve|reject|pause|unpause)$/)?.[1];
    if (req.method === "POST" && action) {
      if (!sameOrigin(req)) return send(res, 403, { error: "cross-site request blocked" });
      const body = await readJson(req);
      if ((action === "approve" || action === "reject") && !/^\d+$/.test(body.id ?? "")) {
        return send(res, 400, { error: "missing request id" });
      }
      const [method, args] = ACTIONS[action](body.id);
      const owner = loadAccount(config.owner);
      const { result, txHash } = await call<unknown>(owner, method, args, 1n); // 1 yocto = full-access key only
      cache = null;
      return send(res, 200, { ok: true, result, tx: `${config.explorer}/${txHash}` });
    }
    send(res, 404, { error: "not found" });
  } catch (err) {
    const msg = (err as Error).message;
    send(res, /429|rate limit/i.test(msg) ? 503 : 500, { error: msg.slice(0, 300) });
  }
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`AgentGuard owner console: http://localhost:${PORT}`);
  console.log(`Vault ${config.vault}, signing as owner ${config.owner}. Ctrl+C to stop.`);
});
