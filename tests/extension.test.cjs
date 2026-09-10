// Dependency-free browser API mocks. Run: node --test tests/extension.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const vm = require("node:vm");

function source(file) {
  return readFileSync(join(__dirname, "..", "extension", file), "utf8");
}

function popup() {
  class Element {
    constructor() {
      this.children = [];
      this.listeners = {};
      this.disabled = false;
      this.textContent = "";
      this.value = "";
    }
    addEventListener(event, handler) {
      this.listeners[event] = handler;
    }
    appendChild(child) {
      this.children.push(child);
    }
    click() {
      this.listeners.click?.();
    }
  }
  const elements = new Map();
  const callbacks = [];
  const timers = [];
  const runtime = {
    lastError: null,
    sendMessage(message, callback) {
      if (message.action === "download") callbacks.push(callback);
      else if (message.action === "ping") callback({ success: true });
      else if (message.action === "getConfig") callback({});
    },
  };
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element());
      return elements.get(id);
    },
    createElement: () => new Element(),
    addEventListener: (_, ready) => ready(),
  };
  vm.runInNewContext(source("popup.js"), {
    document,
    URL,
    chrome: {
      runtime,
      tabs: {
        query: (_, callback) =>
          callback([{ id: 1, url: "https://example.com/page" }]),
        sendMessage: (_, __, callback) =>
          callback({ videos: ["https://example.com/a.mp4"] }),
      },
    },
    setTimeout: (callback, delay) => {
      timers.push({ callback, delay });
      return timers.length;
    },
    clearTimeout: () => {},
  });
  elements.get("scan-btn").click();
  return {
    button: elements.get("video-list").children[0].children[1],
    feedback: elements.get("action-feedback"),
    callbacks,
    runtime,
    timers,
  };
}

test("scan download stays pending until native acknowledgement, then succeeds", () => {
  const p = popup();
  p.button.click();
  assert.equal(p.button.textContent, "Mengirim…");
  assert.equal(p.button.disabled, true);
  p.button.click();
  assert.equal(p.callbacks.length, 1, "pending request is not sent twice");
  p.callbacks[0]({ success: true });
  assert.equal(p.button.textContent, "✓ Terkirim");
  assert.equal(p.button.disabled, true);
  p.timers.find((timer) => timer.delay === 2000).callback();
  assert.equal(p.button.textContent, "⬇ Unduh");
  assert.equal(p.button.disabled, false);
});

for (const failure of ["rejected", "missing", "transport"]) {
  test(`scan download reports ${failure} response and allows retry`, () => {
    const p = popup();
    p.button.click();
    if (failure === "transport")
      p.runtime.lastError = { message: "Native host unavailable" };
    p.callbacks[0](
      failure === "missing" ? undefined : { success: false, error: "Ditolak" },
    );
    assert.equal(p.button.textContent, "↻ Coba lagi");
    assert.equal(p.button.disabled, false);
    assert.equal(p.feedback.className, "action-feedback err");
    assert.match(p.feedback.textContent, /Gagal/);
    p.runtime.lastError = null;
    p.button.click();
    assert.equal(p.callbacks.length, 2);
  });
}

function background() {
  const badges = [];
  const requests = [];
  const logs = [];
  let onMessage;
  const event = { addListener() {} };
  const context = vm.createContext({
    URL,
    setTimeout: () => 1,
    clearTimeout: () => {},
    console: { log: (...args) => logs.push(args.join(" ")), error() {} },
    chrome: {
      runtime: {
        id: "a".repeat(32),
        lastError: null,
        onInstalled: event,
        onStartup: event,
        onMessage: {
          addListener(handler) {
            onMessage = handler;
          },
        },
        sendNativeMessage: (_, message, callback) =>
          requests.push({ message, callback }),
      },
      storage: {
        local: { get: () => {}, set: () => {} },
        onChanged: event,
      },
      cookies: { getAll: async () => [] },
      downloads: { onCreated: event },
      contextMenus: { onClicked: event },
      action: {
        setBadgeText: ({ text }) => badges.push(text),
        setBadgeBackgroundColor: () => {},
      },
    },
  });
  vm.runInContext(source("background.js"), context);
  return { badges, requests, logs, onMessage, context };
}

for (const outcome of ["success", "rejected", "missing", "transport"]) {
  test(`background badge reflects ${outcome}, not just a completed native callback`, async () => {
    const b = background();
    const response = new Promise((resolve) => {
      assert.equal(
        b.onMessage(
          {
            action: "download",
            url: "https://example.com/a.zip?token=private",
          },
          {},
          resolve,
        ),
        true,
      );
    });
    await new Promise(setImmediate); // flush promises across VM contexts
    assert.deepEqual(b.badges, ["…"]);
    assert.equal(b.requests.length, 1);
    if (outcome === "transport") {
      b.context.chrome.runtime.lastError = { message: "Disconnected" };
    }
    b.requests[0].callback(
      outcome === "missing" ? undefined : { success: outcome === "success" },
    );
    const result = await response;
    assert.equal(result.success, outcome === "success");
    assert.equal(b.badges.at(-1), outcome === "success" ? "⬇" : "!");
    assert.ok(b.logs.every((line) => !line.includes("private")));
  });
}


test("background exports cookie attributes and ignores flattened caller cookies", async () => {
  const b = background();
  b.context.chrome.cookies.getAll = async () => [
    { domain: "example.com", name: "sid", value: "private", path: "/login", secure: true,
      hostOnly: true, httpOnly: true, session: false, expirationDate: 2000000000 },
    { domain: "example.com", name: "partition", value: "omit", partitionKey: { topLevelSite: "https://other.test" } },
  ];
  const response = new Promise(resolve => b.onMessage({ action: "download",
    url: "https://example.com/login/file.zip", cookies: "untrusted=wrong", domain: "other.test" }, {}, resolve));
  await new Promise(setImmediate);
  const message = b.requests[0].message;
  assert.equal(message.cookies, undefined);
  assert.equal(message.cookie_jar.length, 1);
  assert.equal(message.cookie_jar[0].secure, true);
  assert.equal(message.cookie_jar[0].hostOnly, true);
  assert.equal(message.cookie_jar[0].httpOnly, true);
  assert.equal(message.cookie_jar[0].path, "/login");
  assert.equal(message.cookie_jar[0].expirationDate, 2000000000);
  b.requests[0].callback({ success: true });
  await response;
});


test("disabled extension refuses manual downloads before reading cookies", async () => {
  const b = background();
  b.onMessage({ action: "setConfig", config: { enabled: false } }, {}, () => {});
  let read = false;
  b.context.chrome.cookies.getAll = async () => { read = true; return []; };
  const result = await new Promise(resolve => b.onMessage({ action: "download", url: "https://example.com/a.zip" }, {}, resolve));
  assert.equal(result.success, false);
  assert.equal(read, false);
  assert.equal(b.requests.length, 0);
});
