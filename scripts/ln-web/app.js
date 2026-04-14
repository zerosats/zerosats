const $ = (id) => document.getElementById(id);

const settingsForm = $("settingsForm");
const resultBox = $("resultBox");
const historyList = $("historyList");
const serverMeta = $("serverMeta");
const monitorInfo = $("monitorInfo");
const monitorWallet = $("monitorWallet");
const monitorChannel = $("monitorChannel");
const monitorMeta = $("monitorMeta");
const pollMeta = $("pollMeta");
const fundingMeta = $("fundingMeta");

let busy = false;
let liveRefreshing = false;
let autoRefreshTimer = null;
let pollTimer = null;
let pollMode = null;
let pollTarget = null;

const UI_PREFS_KEY = "zerosats-ln-web-ui";

function loadUiPrefs() {
  try {
    const raw = localStorage.getItem(UI_PREFS_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw);
    if (typeof parsed.autoRefreshEnabled === "boolean") {
      $("autoRefreshEnabled").checked = parsed.autoRefreshEnabled;
    }
    if (parsed.autoRefreshSec) {
      $("autoRefreshSec").value = String(parsed.autoRefreshSec);
    }
    if (parsed.pollIntervalSec) {
      $("pollIntervalSec").value = String(parsed.pollIntervalSec);
    }
  } catch {
    // ignore malformed local prefs
  }
}

function saveUiPrefs() {
  const payload = {
    autoRefreshEnabled: $("autoRefreshEnabled").checked,
    autoRefreshSec: $("autoRefreshSec").value.trim(),
    pollIntervalSec: $("pollIntervalSec").value.trim(),
  };
  localStorage.setItem(UI_PREFS_KEY, JSON.stringify(payload));
}

function setBusy(nextBusy) {
  busy = nextBusy;
  for (const el of document.querySelectorAll("button")) {
    el.disabled = nextBusy;
  }
}

function writeResult(value) {
  resultBox.textContent =
    typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function formSettingsPayload() {
  const formData = new FormData(settingsForm);
  return Object.fromEntries(formData.entries());
}

function applySettings(settings) {
  for (const [key, value] of Object.entries(settings)) {
    const input = settingsForm.querySelector(`[name="${key}"]`);
    if (input) {
      input.value = value ?? "";
    }
  }
}

function renderHistory(items) {
  historyList.innerHTML = "";
  for (const item of items) {
    const li = document.createElement("li");
    const statusClass = item.ok ? "ok" : "bad";
    li.innerHTML = `
      <div><strong>${item.action}</strong> <span class="${statusClass}">${item.ok ? "ok" : "failed"}</span></div>
      <div class="meta">${item.createdAt} • ${item.summary ?? ""}</div>
    `;
    li.addEventListener("click", () => {
      writeResult(item);
    });
    historyList.appendChild(li);
  }
}

function parseJsonSafe(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function parsePositiveInt(value, fallback) {
  const normalized = String(value || "").trim();
  if (!/^\d+$/.test(normalized)) return fallback;
  const parsed = Number.parseInt(normalized, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
}

function compactString(obj) {
  return JSON.stringify(obj, null, 2);
}

function setMonitorText(target, content) {
  target.textContent = content;
}

function summarizeInfo(result) {
  if (!result.ok) {
    return `ERROR\n${result.stderr || "unknown error"}`;
  }
  const data = parseJsonSafe(result.stdout);
  if (!data) {
    return result.stdout || "(empty)";
  }
  return compactString({
    alias: data.alias,
    identity_pubkey: data.identity_pubkey,
    block_height: data.block_height,
    synced_to_chain: data.synced_to_chain,
    synced_to_graph: data.synced_to_graph,
    num_active_channels: data.num_active_channels,
    num_peers: data.num_peers,
  });
}

function summarizeBalance(result) {
  if (!result.ok) {
    return `ERROR\n${result.stderr || "unknown error"}`;
  }
  const data = parseJsonSafe(result.stdout);
  if (!data) {
    return result.stdout || "(empty)";
  }
  return compactString({
    total_balance: data.total_balance,
    confirmed_balance: data.confirmed_balance,
    unconfirmed_balance: data.unconfirmed_balance,
    locked_balance: data.locked_balance,
  });
}

function summarizeChannelBalance(result) {
  if (!result.ok) {
    return `ERROR\n${result.stderr || "unknown error"}`;
  }
  const data = parseJsonSafe(result.stdout);
  if (!data) {
    return result.stdout || "(empty)";
  }
  return compactString({
    balance: data.balance,
    pending_open_balance: data.pending_open_balance,
    local_balance: data.local_balance,
    remote_balance: data.remote_balance,
  });
}

async function api(path, options = {}) {
  const res = await fetch(path, {
    headers: {
      "content-type": "application/json",
    },
    ...options,
  });
  const body = await res.json();
  if (!res.ok || !body.ok) {
    throw new Error(body.error || `request failed: ${res.status}`);
  }
  return body;
}

async function loadState() {
  const body = await api("/api/state");
  applySettings(body.state.settings || {});
  renderHistory(body.state.history || []);
  serverMeta.textContent = "Persisted state loaded from local server storage";
}

async function saveSettings() {
  const settings = formSettingsPayload();
  const body = await api("/api/settings", {
    method: "POST",
    body: JSON.stringify({ settings }),
  });
  applySettings(body.settings);
  writeResult({ ok: true, message: "settings saved", settings: body.settings });
}

async function refreshHistory() {
  const state = await api("/api/state");
  renderHistory(state.state.history || []);
}

async function runAction(action, payload = {}, options = {}) {
  const body = await api("/api/action", {
    method: "POST",
    body: JSON.stringify({
      action,
      payload,
      noHistory: Boolean(options.noHistory),
    }),
  });
  if (!options.quiet) {
    writeResult(body.result);
  }
  if (!options.noHistory && options.refreshHistory !== false) {
    await refreshHistory();
  }
  return body.result;
}

async function withBusy(fn) {
  if (busy) return;
  setBusy(true);
  try {
    await fn();
  } catch (error) {
    writeResult({ ok: false, error: error.message || String(error) });
  } finally {
    setBusy(false);
  }
}

function getPaymentHashFromResult(result) {
  const data = parseJsonSafe(result?.stdout);
  if (!data) return "";
  if (typeof data.payment_hash === "string" && data.payment_hash) {
    return data.payment_hash;
  }
  if (
    data.payment &&
    typeof data.payment.payment_hash === "string" &&
    data.payment.payment_hash
  ) {
    return data.payment.payment_hash;
  }
  return "";
}

function getInvoiceFromAddInvoice(result) {
  const data = parseJsonSafe(result?.stdout);
  if (!data) return "";
  if (typeof data.payment_request === "string" && data.payment_request) {
    return data.payment_request;
  }
  return "";
}

function stopPaymentPolling(reason = "Polling stopped.") {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  pollMode = null;
  pollTarget = null;
  pollMeta.textContent = reason;
}

async function pollOnce() {
  if (!pollMode || !pollTarget) return;
  let result;
  if (pollMode === "pay") {
    result = await runAction(
      "track",
      { paymentHash: pollTarget },
      { noHistory: true, quiet: true, refreshHistory: false },
    );
    const data = parseJsonSafe(result.stdout) || {};
    const status = String(data.status || data.payment_status || "UNKNOWN");
    pollMeta.textContent = `Outgoing payment status: ${status}`;
    if (status === "SUCCEEDED" || status === "FAILED") {
      stopPaymentPolling(`Outgoing payment finished: ${status}`);
      writeResult(result);
    }
    return;
  }
  if (pollMode === "invoice") {
    result = await runAction(
      "lookup-invoice",
      { invoice: pollTarget },
      { noHistory: true, quiet: true, refreshHistory: false },
    );
    const data = parseJsonSafe(result.stdout) || {};
    const settled = Boolean(data.settled);
    const state = String(data.state || (settled ? "SETTLED" : "OPEN"));
    pollMeta.textContent = `Incoming invoice status: ${state}`;
    if (settled || state === "SETTLED" || state === "CANCELED") {
      stopPaymentPolling(`Incoming invoice finished: ${state}`);
      writeResult(result);
    }
  }
}

function startPaymentPolling(mode) {
  const intervalSec = parsePositiveInt($("pollIntervalSec").value, 8);
  if (mode === "pay") {
    const paymentHash = $("paymentHashInput").value.trim();
    if (!paymentHash) {
      throw new Error("payment hash required for outgoing poll");
    }
    pollMode = "pay";
    pollTarget = paymentHash;
    pollMeta.textContent = `Polling outgoing payment every ${intervalSec}s...`;
  } else {
    const invoice = $("invoiceInput").value.trim();
    if (!invoice) {
      throw new Error("invoice required for incoming poll");
    }
    pollMode = "invoice";
    pollTarget = invoice;
    pollMeta.textContent = `Polling incoming invoice every ${intervalSec}s...`;
  }
  if (pollTimer) {
    clearInterval(pollTimer);
  }
  pollTimer = setInterval(() => {
    pollOnce().catch((error) => {
      pollMeta.textContent = `Polling error: ${error.message || String(error)}`;
    });
  }, intervalSec * 1000);
  pollOnce().catch((error) => {
    pollMeta.textContent = `Polling error: ${error.message || String(error)}`;
  });
}

async function refreshLiveStatus() {
  if (liveRefreshing || busy) return;
  liveRefreshing = true;
  const startedAt = new Date();
  try {
    const [infoRes, walletRes, channelRes] = await Promise.all([
      runAction("getinfo", {}, { noHistory: true, quiet: true, refreshHistory: false }),
      runAction("wallet-balance", {}, { noHistory: true, quiet: true, refreshHistory: false }),
      runAction("channel-balance", {}, { noHistory: true, quiet: true, refreshHistory: false }),
    ]);
    setMonitorText(monitorInfo, summarizeInfo(infoRes));
    setMonitorText(monitorWallet, summarizeBalance(walletRes));
    setMonitorText(monitorChannel, summarizeChannelBalance(channelRes));
    monitorMeta.textContent = `Last refresh: ${startedAt.toLocaleTimeString()}`;
  } catch (error) {
    monitorMeta.textContent = `Monitor error: ${error.message || String(error)}`;
  } finally {
    liveRefreshing = false;
  }
}

function restartAutoRefreshIfEnabled() {
  if (autoRefreshTimer) {
    clearInterval(autoRefreshTimer);
    autoRefreshTimer = null;
  }
  if (!$("autoRefreshEnabled").checked) {
    monitorMeta.textContent = "Auto refresh is off.";
    saveUiPrefs();
    return;
  }
  const intervalSec = parsePositiveInt($("autoRefreshSec").value, 20);
  autoRefreshTimer = setInterval(() => {
    refreshLiveStatus();
  }, intervalSec * 1000);
  monitorMeta.textContent = `Auto refresh every ${intervalSec}s.`;
  saveUiPrefs();
}

$("saveSettingsBtn").addEventListener("click", () => withBusy(saveSettings));
$("doctorBtn").addEventListener("click", () =>
  withBusy(() => runAction("doctor")),
);
$("getInfoBtn").addEventListener("click", () =>
  withBusy(() => runAction("getinfo")),
);

for (const button of document.querySelectorAll("button[data-action]")) {
  const action = button.dataset.action;
  button.addEventListener("click", () => withBusy(() => runAction(action)));
}

$("createInvoiceBtn").addEventListener("click", () =>
  withBusy(async () => {
    const amountSat = $("invoiceAmount").value.trim();
    const memo = $("invoiceMemo").value.trim();
    const result = await runAction("add-invoice", { amountSat, memo });
    const invoice = getInvoiceFromAddInvoice(result);
    if (invoice) {
      $("invoiceInput").value = invoice;
      pollMeta.textContent = "Invoice created. You can start incoming poll.";
    }
  }),
);

$("decodeBtn").addEventListener("click", () =>
  withBusy(async () => {
    const invoice = $("invoiceInput").value.trim();
    const result = await runAction("decode", { invoice });
    const hash = getPaymentHashFromResult(result);
    if (hash) {
      $("paymentHashInput").value = hash;
    }
  }),
);

$("payBtn").addEventListener("click", () =>
  withBusy(async () => {
    const invoice = $("invoiceInput").value.trim();
    const feeLimitSat = $("payFeeLimit").value.trim();
    const timeoutSeconds = $("payTimeout").value.trim();
    const result = await runAction("pay", { invoice, feeLimitSat, timeoutSeconds });
    const hash = getPaymentHashFromResult(result);
    if (hash) {
      $("paymentHashInput").value = hash;
      pollMeta.textContent =
        "Outgoing payment submitted. You can start outgoing poll.";
    }
  }),
);

$("trackBtn").addEventListener("click", () =>
  withBusy(async () => {
    const paymentHash = $("paymentHashInput").value.trim();
    await runAction("track", { paymentHash });
  }),
);

$("startPayPollBtn").addEventListener("click", () =>
  withBusy(async () => {
    startPaymentPolling("pay");
  }),
);

$("startInvoicePollBtn").addEventListener("click", () =>
  withBusy(async () => {
    startPaymentPolling("invoice");
  }),
);

$("stopPollBtn").addEventListener("click", () =>
  withBusy(async () => {
    stopPaymentPolling("Polling stopped by user.");
  }),
);

$("refreshNowBtn").addEventListener("click", () =>
  withBusy(async () => {
    await refreshLiveStatus();
  }),
);

$("autoRefreshEnabled").addEventListener("change", () => {
  restartAutoRefreshIfEnabled();
});

$("autoRefreshSec").addEventListener("change", () => {
  restartAutoRefreshIfEnabled();
});

$("pollIntervalSec").addEventListener("change", () => {
  saveUiPrefs();
});

$("addPeerBtn").addEventListener("click", () =>
  withBusy(async () => {
    const target = $("addPeerTarget").value.trim();
    await runAction("add-peer", { target });
  }),
);

$("openChannelBtn").addEventListener("click", () =>
  withBusy(async () => {
    const nodePubkey = $("openNodePubkey").value.trim();
    const localAmtSat = $("openLocalAmt").value.trim();
    const pushAmtSat = $("openPushAmt").value.trim();
    const satPerVbyte = $("openSatPerVbyte").value.trim();
    await runAction("open-channel", {
      nodePubkey,
      localAmtSat,
      pushAmtSat,
      satPerVbyte,
    });
  }),
);

$("generateAddressBtn").addEventListener("click", () =>
  withBusy(async () => {
    const result = await runAction("new-address", {});
    const data = parseJsonSafe(result.stdout) || {};
    const address = String(data.address || "").trim();
    if (address) {
      $("fundingAddressOut").value = address;
      fundingMeta.textContent =
        "Address generated. Send testnet coins here, then click Check Funded.";
    } else {
      fundingMeta.textContent =
        "Address generation returned no address field. See Latest Result.";
    }
  }),
);

$("copyAddressBtn").addEventListener("click", () =>
  withBusy(async () => {
    const address = $("fundingAddressOut").value.trim();
    if (!address) {
      throw new Error("no funding address available");
    }
    await navigator.clipboard.writeText(address);
    fundingMeta.textContent = "Funding address copied to clipboard.";
  }),
);

$("checkFundedBtn").addEventListener("click", () =>
  withBusy(async () => {
    const minTarget = parsePositiveInt($("fundingTargetSat").value, 100000);
    const result = await runAction(
      "wallet-balance",
      {},
      { noHistory: true, quiet: true, refreshHistory: false },
    );
    const data = parseJsonSafe(result.stdout) || {};
    const confirmed = Number.parseInt(String(data.confirmed_balance || "0"), 10);
    if (Number.isFinite(confirmed) && confirmed >= minTarget) {
      fundingMeta.textContent = `Funded: confirmed ${confirmed} sats (target ${minTarget}).`;
    } else {
      fundingMeta.textContent = `Not funded yet: confirmed ${Number.isFinite(confirmed) ? confirmed : 0} sats (target ${minTarget}).`;
    }
    writeResult(result);
    await refreshHistory();
  }),
);

$("clearHistoryBtn").addEventListener("click", () =>
  withBusy(async () => {
    await api("/api/history/clear", { method: "POST", body: "{}" });
    renderHistory([]);
    writeResult({ ok: true, message: "history cleared" });
  }),
);

loadUiPrefs();
loadState()
  .then(() => refreshLiveStatus())
  .then(() => restartAutoRefreshIfEnabled())
  .catch((error) => {
    writeResult({ ok: false, error: error.message || String(error) });
  });
