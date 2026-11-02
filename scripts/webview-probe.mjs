#!/usr/bin/env node
// What each Voltip webview is running, read over the Chrome DevTools Protocol of WebView2
// (`--remote-debugging-port`, scripts/windows-upgrade-probe.ps1 enables it): the page's URL, the
// window's label, the entry script the document loaded, how the document and its scripts were
// delivered, and for the main window the app version, the relay's state as the window shows it
// and whether the dictionary page has its 学习建议 tab. Prints one JSON object on stdout.
//
//   node scripts/webview-probe.mjs --port 9222 [--wait-ms 60000] [--relay-connected-ms 0]
//     [--guard-check 1]
//
// `--wait-ms`: how long to wait for the main window and the pill window to have rendered.
// `--relay-connected-ms`: when non-zero, also wait that long for the window's relay card to read
// connected (the answer records how long it took, or that it never did).
// `--guard-check 1`: afterwards, check the main window's interface check (docs/dictation.md
// §33.4) on the running program: reload the window with a document that names an entry script the
// program does not carry, count the documents it goes through, then reload it with its own.
// Node 22 or later (global fetch and WebSocket). Never writes to the page except to open the
// dictionary page in the main window and, for the guard check, to reload it.

const DEFAULT_WAIT_MS = 60_000;
const EVALUATE_TIMEOUT_MS = 30_000;
/** The key `ui_guard.js` keeps its once in. */
const GUARD_KEY = "voltip.ui-guard.reloaded";
/** After a document comes in, how long no other one may follow for the window to count as settled:
 *  the check reloads at DOMContentLoaded, well inside this. */
const SETTLED_MS = 3_000;

function parseArgs(argv) {
  const options = { port: 0, waitMs: DEFAULT_WAIT_MS, relayConnectedMs: 0, guardCheck: false };
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    if (flag === "--port") options.port = Number(value);
    else if (flag === "--wait-ms") options.waitMs = Number(value);
    else if (flag === "--relay-connected-ms") options.relayConnectedMs = Number(value);
    else if (flag === "--guard-check") options.guardCheck = value === "1";
    else throw new Error(`unknown flag ${flag}`);
  }
  if (!Number.isInteger(options.port) || options.port <= 0) throw new Error("--port is required");
  return options;
}

/** The page targets the debugging port lists. */
async function pages(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`);
  if (!response.ok) throw new Error(`/json/list answered ${response.status}`);
  const all = await response.json();
  return all.filter((t) => t.type === "page" && typeof t.webSocketDebuggerUrl === "string");
}

/** One CDP connection: `call(method, params)` resolves with the command's result, `on(method,
 *  handler)` hears its events. */
async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error(`cannot connect to ${url}`)), {
      once: true,
    });
  });
  let next = 0;
  const waiting = new Map();
  const listeners = new Map();
  socket.addEventListener("message", (message) => {
    const data = JSON.parse(String(message.data));
    if (data.id === undefined) {
      for (const handler of listeners.get(data.method) ?? []) handler(data.params ?? {});
      return;
    }
    const waiter = waiting.get(data.id);
    if (!waiter) return;
    waiting.delete(data.id);
    if (data.error) waiter.reject(new Error(`${data.error.message} (${data.error.code})`));
    else waiter.resolve(data.result);
  });
  const call = (method, params = {}) =>
    new Promise((resolve, reject) => {
      next += 1;
      const id = next;
      const timer = setTimeout(() => {
        waiting.delete(id);
        reject(new Error(`${method} timed out`));
      }, EVALUATE_TIMEOUT_MS);
      waiting.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      socket.send(JSON.stringify({ id, method, params }));
    });
  const on = (method, handler) => {
    if (!listeners.has(method)) listeners.set(method, []);
    listeners.get(method).push(handler);
  };
  return { call, on, close: () => socket.close() };
}

/** Evaluate `source` (an async function body that reads `WAIT_MS` and `RELAY_MS`) in the page
 *  and return its JSON value. */
async function evaluate(session, source, options) {
  const constants = `const WAIT_MS = ${options.waitMs}; const RELAY_MS = ${options.relayConnectedMs};`;
  const result = await session.call("Runtime.evaluate", {
    expression: `(async () => { ${constants} ${source} })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    const text = result.exceptionDetails.exception?.description ?? result.exceptionDetails.text;
    throw new Error(`the page threw: ${text}`);
  }
  return result.result.value;
}

// In-page helpers, shared by the expressions below.
const HELPERS = `
  const until = (probe, ms) => new Promise((resolve) => {
    const started = Date.now();
    const tick = () => {
      const value = probe();
      if (value || Date.now() - started >= ms) resolve({ value, ms: Date.now() - started });
      else setTimeout(tick, 100);
    };
    tick();
  });
  const path = (url) => { try { return new URL(url, location.href).pathname; } catch { return String(url); } };
`;

/** What every window reports: where it is, what it loaded and how. */
const DESCRIBE = `
  ${HELPERS}
  const label = window.__TAURI_INTERNALS__?.metadata?.currentWebview?.label ?? null;
  const rendered = await until(() => (document.getElementById("root")?.childElementCount ?? 0) > 0, WAIT_MS);
  const navigation = performance.getEntriesByType("navigation")[0];
  const scripts = performance.getEntriesByType("resource")
    .filter((e) => e.initiatorType === "script" || path(e.name).endsWith(".js"))
    .map((e) => ({ path: path(e.name), transferSize: e.transferSize, deliveryType: e.deliveryType ?? "" }));
  return {
    href: location.href,
    // When this document started loading (Unix ms): a document older than its process is stale.
    timeOrigin: Math.round(performance.timeOrigin),
    label,
    // WebView2 reports "visible" for the webview of a hidden window too: whether a window is shown
    // is read from Windows (scripts/windows-upgrade-probe.ps1).
    visibility: document.visibilityState,
    entry: [...document.querySelectorAll("script[type=module][src]")].map((s) => path(s.getAttribute("src"))),
    scripts,
    navigation: navigation ? { type: navigation.type, deliveryType: navigation.deliveryType ?? "", transferSize: navigation.transferSize } : null,
    rendered: rendered.value === true,
    renderedMs: rendered.ms,
    // Set while the interface check has reloaded this window and it still runs other code.
    guardKey: (() => { try { return sessionStorage.getItem(${JSON.stringify(GUARD_KEY)}); } catch { return null; } })(),
  };
`;

/** The main window: the app version and relay the core reports, and what the window shows. */
const MAIN = `
  ${HELPERS}
  const internals = window.__TAURI_INTERNALS__;
  let core = null;
  try {
    const state = await internals.invoke("core_state");
    core = { appVersion: state.app_version, relay: state.relay?.state ?? null };
  } catch (e) { core = { error: String(e) }; }
  const card = () => document.querySelector('[data-testid="sidebar-status-card"]')?.innerText?.trim() ?? null;
  const shown = await until(() => card(), WAIT_MS);
  let relayCard = { text: shown.value, connectedMs: null };
  if (RELAY_MS > 0) {
    const connected = await until(() => {
      const text = card();
      return text !== null && (text.includes("已连接") || /(^|[^a-z])connected/i.test(text)) ? text : null;
    }, RELAY_MS);
    relayCard = { text: connected.value ?? card(), connectedMs: connected.value ? connected.ms : null };
  }
  location.hash = "#/dictionary";
  const page = await until(() => document.querySelector('[data-testid="page-dictionary"]'), WAIT_MS);
  const tabs = [...document.querySelectorAll('[data-testid="page-dictionary"] [role=tab]')].map((t) => t.textContent.trim());
  return {
    core,
    relayCard,
    dictionary: {
      page: page.value !== null && page.value !== undefined,
      suggestionsTab: document.getElementById("dictionary-tab-suggestions") !== null,
      tabs,
    },
  };
`;

/** Run in every new document of the window under the guard check: once the parser has inserted a
 *  module script loading an `index-….js`, it names one the program does not carry. The browser
 *  has already fetched the program's own script; the check reads the document. */
const STALE_DOCUMENT = `(() => {
  const stale = (node) => {
    if (!(node instanceof HTMLScriptElement) || node.type !== "module") return;
    const src = node.getAttribute("src") ?? "";
    if (/\\/assets\\/index-[^/]+\\.js$/.test(src) && !src.includes("STALE")) node.setAttribute("src", "/assets/index-STALE000.js");
  };
  new MutationObserver((records) => {
    for (const record of records) for (const node of record.addedNodes) stale(node);
  }).observe(document, { childList: true, subtree: true });
})();`;

/** What the window shows once settled: its entry scripts and the check's key. */
const SETTLED = `
  ${HELPERS}
  await until(() => document.readyState === "complete", WAIT_MS);
  return {
    entry: [...document.querySelectorAll("script[type=module][src]")].map((s) => path(s.getAttribute("src"))),
    navigation: performance.getEntriesByType("navigation")[0]?.type ?? null,
    guardKey: (() => { try { return sessionStorage.getItem(${JSON.stringify(GUARD_KEY)}); } catch { return null; } })(),
  };
`;

/** Reload the main window and report every document it goes through until none follows for
 *  `SETTLED_MS`: the commits counted, the check's console lines, what the last document shows. */
async function reloadAndSettle(session, seen, options) {
  const from = { documents: seen.documents.length, warnings: seen.warnings.length };
  await session.call("Page.reload", {});
  const deadline = Date.now() + options.waitMs;
  for (;;) {
    const last = seen.documents.at(-1);
    if (seen.documents.length > from.documents && Date.now() - last >= SETTLED_MS) break;
    if (Date.now() >= deadline) throw new Error(`the window did not settle within ${options.waitMs} ms`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return {
    documents: seen.documents.length - from.documents,
    warnings: seen.warnings.slice(from.warnings),
    ...(await evaluate(session, SETTLED, options)),
  };
}

/** The interface check on the running program (docs/dictation.md §33.4), in the main window. */
async function guardCheck(target, options) {
  const session = await connect(target.webSocketDebuggerUrl);
  const seen = { documents: [], warnings: [] };
  try {
    session.on("Page.frameNavigated", ({ frame }) => {
      if (frame && frame.parentId === undefined) seen.documents.push(Date.now());
    });
    session.on("Runtime.consoleAPICalled", ({ args }) => {
      const text = (args ?? []).map((a) => (a.value === undefined ? (a.description ?? "") : JSON.stringify(a.value))).join(" ");
      if (text.startsWith('"voltip:')) seen.warnings.push(text);
    });
    await session.call("Page.enable", {});
    await session.call("Runtime.enable", {});
    const { identifier } = await session.call("Page.addScriptToEvaluateOnNewDocument", { source: STALE_DOCUMENT });
    const stale = await reloadAndSettle(session, seen, options);
    await session.call("Page.removeScriptToEvaluateOnNewDocument", { identifier });
    const own = await reloadAndSettle(session, seen, options);
    return { stale, own };
  } finally {
    session.close();
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const out = { port: options.port, windows: [] };
  for (const target of await pages(options.port)) {
    const session = await connect(target.webSocketDebuggerUrl);
    try {
      const window = { targetUrl: target.url, ...(await evaluate(session, DESCRIBE, options)) };
      const main = window.label === "main" || (window.label === null && !target.url.includes("overlay"));
      if (main) Object.assign(window, await evaluate(session, MAIN, options));
      out.windows.push(window);
      if (main && options.guardCheck) {
        session.close();
        try {
          out.guard = await guardCheck(target, options);
        } catch (e) {
          out.guard = { error: String(e instanceof Error ? e.message : e) };
        }
      }
    } catch (e) {
      out.windows.push({ targetUrl: target.url, error: String(e instanceof Error ? e.message : e) });
    } finally {
      session.close();
    }
  }
  process.stdout.write(`${JSON.stringify(out)}\n`);
}

main().catch((e) => {
  process.stderr.write(`webview-probe: ${e instanceof Error ? e.message : String(e)}\n`);
  process.exitCode = 1;
});
