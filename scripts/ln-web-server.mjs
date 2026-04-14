#!/usr/bin/env node
import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const HOST = process.env.LN_WEB_HOST || "127.0.0.1";
const PORT = Number.parseInt(process.env.LN_WEB_PORT || "8788", 10);
const DATA_DIR =
  process.env.LN_WEB_DATA_DIR || path.join(os.homedir(), ".zerosats-ln-web");
const STATE_FILE = path.join(DATA_DIR, "state.json");
const STATIC_DIR = path.join(__dirname, "ln-web");
const MAX_HISTORY_ITEMS = 200;

const DEFAULT_SETTINGS = Object.freeze({
  lncliBin: "lncli",
  network: "testnet",
  rpcServer: "",
  macaroonPath: "",
  tlsCertPath: "",
  defaultFeeLimitSat: "200",
  defaultTimeoutSeconds: "120",
  defaultMemo: "zerosats-ln-web",
});

const DEFAULT_STATE = Object.freeze({
  settings: DEFAULT_SETTINGS,
  history: [],
});

const CONTENT_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
};

let state = await loadState();

function nowIso() {
  return new Date().toISOString();
}

function mergeSettings(incoming = {}) {
  return {
    ...DEFAULT_SETTINGS,
    ...incoming,
  };
}

async function loadState() {
  await fs.mkdir(DATA_DIR, { recursive: true });
  try {
    const raw = await fs.readFile(STATE_FILE, "utf8");
    const parsed = JSON.parse(raw);
    return {
      settings: mergeSettings(parsed.settings),
      history: Array.isArray(parsed.history) ? parsed.history : [],
    };
  } catch (error) {
    if (error && error.code !== "ENOENT") {
      console.error(`[${nowIso()}] state load failed:`, error);
    }
    const fresh = {
      settings: { ...DEFAULT_SETTINGS },
      history: [],
    };
    await saveState(fresh);
    return fresh;
  }
}

async function saveState(nextState = state) {
  const serialized = JSON.stringify(nextState, null, 2);
  await fs.writeFile(STATE_FILE, serialized, "utf8");
}

function jsonResponse(res, statusCode, body) {
  const payload = JSON.stringify(body);
  res.writeHead(statusCode, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  res.end(payload);
}

async function readJsonBody(req) {
  const chunks = [];
  for await (const chunk of req) {
    chunks.push(chunk);
  }
  const text = Buffer.concat(chunks).toString("utf8").trim();
  if (!text) {
    return {};
  }
  return JSON.parse(text);
}

function sanitizeHistoryEntry(entry) {
  return {
    id: entry.id,
    createdAt: entry.createdAt,
    action: entry.action,
    request: entry.request,
    ok: entry.ok,
    summary: entry.summary,
    stdout: entry.stdout,
    stderr: entry.stderr,
    exitCode: entry.exitCode,
  };
}

async function addHistory(entry) {
  state.history.unshift(entry);
  if (state.history.length > MAX_HISTORY_ITEMS) {
    state.history.length = MAX_HISTORY_ITEMS;
  }
  await saveState();
}

function buildBaseArgs(settings) {
  const args = [];
  args.push(`--network=${settings.network || "testnet"}`);
  if (settings.rpcServer) {
    args.push(`--rpcserver=${settings.rpcServer}`);
  }
  if (settings.macaroonPath) {
    args.push(`--macaroonpath=${settings.macaroonPath}`);
  }
  if (settings.tlsCertPath) {
    args.push(`--tlscertpath=${settings.tlsCertPath}`);
  }
  return args;
}

function parsePositiveInt(value, name) {
  if (value === undefined || value === null || value === "") {
    return null;
  }
  const normalized = String(value).trim();
  if (!/^\d+$/.test(normalized)) {
    throw new Error(`${name} must be a positive integer`);
  }
  const parsed = Number.parseInt(normalized, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be > 0`);
  }
  return parsed;
}

async function runLncli(settings, commandArgs) {
  const bin = settings.lncliBin || "lncli";
  const args = [...buildBaseArgs(settings), ...commandArgs];

  return await new Promise((resolve) => {
    const child = spawn(bin, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    child.on("error", (error) => {
      resolve({
        ok: false,
        stdout: "",
        stderr: `${error.message}`,
        exitCode: -1,
      });
    });
    child.on("close", (code) => {
      resolve({
        ok: code === 0,
        stdout: stdout.trim(),
        stderr: stderr.trim(),
        exitCode: code ?? -1,
      });
    });
  });
}

function summaryForAction(action, result) {
  if (!result.ok) {
    return `${action} failed`;
  }
  if (action === "pay") {
    return "payment submitted";
  }
  if (action === "add-invoice") {
    return "invoice created";
  }
  if (action === "open-channel") {
    return "channel open requested";
  }
  if (action === "new-address") {
    return "new on-chain address created";
  }
  if (action === "lookup-invoice") {
    return "invoice lookup success";
  }
  return `${action} success`;
}

function parseJsonSafe(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function runLookupInvoice(settings, paymentHash) {
  const attempts = [
    ["lookupinvoice", `--rhash=${paymentHash}`, "--output", "json"],
    ["lookupinvoice", `--r_hash=${paymentHash}`, "--output", "json"],
    ["lookupinvoice", paymentHash, "--output", "json"],
  ];
  let last = null;
  for (const args of attempts) {
    const result = await runLncli(settings, args);
    if (result.ok) {
      return result;
    }
    last = result;
  }
  return (
    last || {
      ok: false,
      stdout: "",
      stderr: "lookupinvoice failed",
      exitCode: -1,
    }
  );
}

async function runAction(action, payload = {}, options = {}) {
  const s = state.settings;
  let commandArgs;
  let result = null;

  switch (action) {
    case "doctor":
    case "getinfo":
      commandArgs = ["getinfo", "--output", "json"];
      break;
    case "wallet-balance":
      commandArgs = ["walletbalance", "--output", "json"];
      break;
    case "channel-balance":
      commandArgs = ["channelbalance", "--output", "json"];
      break;
    case "channels":
      commandArgs = ["listchannels", "--output", "json"];
      break;
    case "peers":
      commandArgs = ["listpeers", "--output", "json"];
      break;
    case "list-payments":
      commandArgs = ["listpayments", "--output", "json"];
      break;
    case "new-address":
      commandArgs = ["newaddress", "p2wkh", "--output", "json"];
      break;
    case "decode": {
      const invoice = String(payload.invoice || "").trim();
      if (!invoice) {
        throw new Error("invoice is required");
      }
      commandArgs = ["decodepayreq", invoice, "--output", "json"];
      break;
    }
    case "lookup-invoice": {
      let paymentHash = String(payload.paymentHash || "").trim();
      if (!paymentHash) {
        const invoice = String(payload.invoice || "").trim();
        if (!invoice) {
          throw new Error("paymentHash or invoice is required");
        }
        const decoded = await runLncli(s, ["decodepayreq", invoice, "--output", "json"]);
        if (!decoded.ok) {
          throw new Error(decoded.stderr || "decodepayreq failed");
        }
        const decodedJson = parseJsonSafe(decoded.stdout);
        paymentHash = String(decodedJson?.payment_hash || "").trim();
        if (!paymentHash) {
          throw new Error("could not derive payment_hash from invoice");
        }
      }
      result = await runLookupInvoice(s, paymentHash);
      break;
    }
    case "pay": {
      const invoice = String(payload.invoice || "").trim();
      if (!invoice) {
        throw new Error("invoice is required");
      }
      const feeLimitSat =
        parsePositiveInt(payload.feeLimitSat, "feeLimitSat") ??
        parsePositiveInt(s.defaultFeeLimitSat, "defaultFeeLimitSat") ??
        200;
      const timeoutSeconds =
        parsePositiveInt(payload.timeoutSeconds, "timeoutSeconds") ??
        parsePositiveInt(s.defaultTimeoutSeconds, "defaultTimeoutSeconds") ??
        120;
      commandArgs = [
        "payinvoice",
        `--pay_req=${invoice}`,
        `--fee_limit_sat=${feeLimitSat}`,
        `--timeout_seconds=${timeoutSeconds}`,
        "--force",
        "--output",
        "json",
      ];
      break;
    }
    case "track": {
      const paymentHash = String(payload.paymentHash || "").trim();
      if (!paymentHash) {
        throw new Error("paymentHash is required");
      }
      commandArgs = ["trackpayment", paymentHash, "--output", "json"];
      break;
    }
    case "add-invoice": {
      const amountSat = parsePositiveInt(payload.amountSat, "amountSat");
      if (!amountSat) {
        throw new Error("amountSat is required");
      }
      const memo = String(payload.memo || s.defaultMemo || "").trim();
      commandArgs = [
        "addinvoice",
        `--amt=${amountSat}`,
        ...(memo ? [`--memo=${memo}`] : []),
        "--output",
        "json",
      ];
      break;
    }
    case "add-peer": {
      const target = String(payload.target || "").trim();
      if (!target) {
        throw new Error("target (pubkey@host:port) is required");
      }
      commandArgs = ["connect", target, "--output", "json"];
      break;
    }
    case "open-channel": {
      const nodePubkey = String(payload.nodePubkey || "").trim();
      if (!nodePubkey) {
        throw new Error("nodePubkey is required");
      }
      const localAmt = parsePositiveInt(payload.localAmtSat, "localAmtSat");
      if (!localAmt) {
        throw new Error("localAmtSat is required");
      }
      const pushAmt = parsePositiveInt(payload.pushAmtSat, "pushAmtSat");
      const satPerVbyte = parsePositiveInt(payload.satPerVbyte, "satPerVbyte");
      commandArgs = [
        "openchannel",
        `--node_key=${nodePubkey}`,
        `--local_amt=${localAmt}`,
        ...(pushAmt ? [`--push_amt=${pushAmt}`] : []),
        ...(satPerVbyte ? [`--sat_per_vbyte=${satPerVbyte}`] : []),
        "--output",
        "json",
      ];
      break;
    }
    default:
      throw new Error(`unknown action: ${action}`);
  }

  if (!result) {
    result = await runLncli(s, commandArgs);
  }
  const entry = sanitizeHistoryEntry({
    id: `${Date.now()}-${Math.random().toString(16).slice(2, 8)}`,
    createdAt: nowIso(),
    action,
    request: payload,
    ok: result.ok,
    summary: summaryForAction(action, result),
    stdout: result.stdout,
    stderr: result.stderr,
    exitCode: result.exitCode,
  });
  if (!options.noHistory) {
    await addHistory(entry);
  }
  return {
    ...result,
    entry: options.noHistory ? undefined : entry,
  };
}

async function handleApi(req, res) {
  if (req.method === "GET" && req.url === "/api/state") {
    return jsonResponse(res, 200, {
      ok: true,
      state,
    });
  }

  if (req.method === "POST" && req.url === "/api/settings") {
    try {
      const body = await readJsonBody(req);
      state.settings = mergeSettings(body.settings || {});
      await saveState();
      return jsonResponse(res, 200, { ok: true, settings: state.settings });
    } catch (error) {
      return jsonResponse(res, 400, {
        ok: false,
        error: error.message || "invalid settings payload",
      });
    }
  }

  if (req.method === "POST" && req.url === "/api/action") {
    try {
      const body = await readJsonBody(req);
      const action = String(body.action || "").trim();
      if (!action) {
        throw new Error("action is required");
      }
      const result = await runAction(action, body.payload || {}, {
        noHistory: Boolean(body.noHistory),
      });
      return jsonResponse(res, 200, { ok: true, result });
    } catch (error) {
      return jsonResponse(res, 400, {
        ok: false,
        error: error.message || "action failed",
      });
    }
  }

  if (req.method === "POST" && req.url === "/api/history/clear") {
    state.history = [];
    await saveState();
    return jsonResponse(res, 200, { ok: true });
  }

  return false;
}

async function serveStatic(req, res) {
  const original = req.url || "/";
  const pathname = original.split("?")[0];
  const normalized = pathname === "/" ? "/index.html" : pathname;
  const target = path.normalize(path.join(STATIC_DIR, normalized));

  if (!target.startsWith(STATIC_DIR)) {
    res.writeHead(403);
    res.end("forbidden");
    return;
  }

  try {
    const content = await fs.readFile(target);
    const ext = path.extname(target);
    const contentType =
      CONTENT_TYPES[ext] || "application/octet-stream; charset=utf-8";
    res.writeHead(200, {
      "content-type": contentType,
      "cache-control": "no-store",
    });
    res.end(content);
  } catch {
    res.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    res.end("not found");
  }
}

const server = http.createServer(async (req, res) => {
  try {
    if ((req.url || "").startsWith("/api/")) {
      const handled = await handleApi(req, res);
      if (handled !== false) {
        return;
      }
      res.writeHead(404, { "content-type": "application/json; charset=utf-8" });
      res.end(JSON.stringify({ ok: false, error: "not-found" }));
      return;
    }
    await serveStatic(req, res);
  } catch (error) {
    res.writeHead(500, { "content-type": "application/json; charset=utf-8" });
    res.end(
      JSON.stringify({
        ok: false,
        error: error.message || "internal-server-error",
      }),
    );
  }
});

server.listen(PORT, HOST, () => {
  console.log(`ln-web listening on http://${HOST}:${PORT}`);
  console.log(`state file: ${STATE_FILE}`);
});
