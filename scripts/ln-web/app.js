const $ = (id) => document.getElementById(id);

const settingsForm = $("settingsForm");
const resultBox = $("resultBox");
const historyList = $("historyList");
const serverMeta = $("serverMeta");

let busy = false;

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

async function runAction(action, payload = {}) {
  const body = await api("/api/action", {
    method: "POST",
    body: JSON.stringify({ action, payload }),
  });
  writeResult(body.result);
  const state = await api("/api/state");
  renderHistory(state.state.history || []);
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
    await runAction("add-invoice", { amountSat, memo });
  }),
);

$("decodeBtn").addEventListener("click", () =>
  withBusy(async () => {
    const invoice = $("invoiceInput").value.trim();
    await runAction("decode", { invoice });
  }),
);

$("payBtn").addEventListener("click", () =>
  withBusy(async () => {
    const invoice = $("invoiceInput").value.trim();
    const feeLimitSat = $("payFeeLimit").value.trim();
    const timeoutSeconds = $("payTimeout").value.trim();
    await runAction("pay", { invoice, feeLimitSat, timeoutSeconds });
  }),
);

$("trackBtn").addEventListener("click", () =>
  withBusy(async () => {
    const paymentHash = $("paymentHashInput").value.trim();
    await runAction("track", { paymentHash });
  }),
);

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

$("clearHistoryBtn").addEventListener("click", () =>
  withBusy(async () => {
    await api("/api/history/clear", { method: "POST", body: "{}" });
    renderHistory([]);
    writeResult({ ok: true, message: "history cleared" });
  }),
);

loadState().catch((error) => {
  writeResult({ ok: false, error: error.message || String(error) });
});
