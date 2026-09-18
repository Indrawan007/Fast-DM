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

function background(cookieJar = [], sniffedCandidates = []) {
  const badges = [];
  const requests = [];
  const logs = [];
  const alarmCreates = [];
  const alarmClears = [];
  let onMessage;
  let onContextMenu;
  let onAlarm;
  const event = { addListener() {} };
  const contextMenuEvent = {
    addListener(handler) {
      onContextMenu = handler;
    },
  };
  const alarmEvent = {
    addListener(handler) {
      onAlarm = handler;
    },
  };
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
      alarms: {
        onAlarm: alarmEvent,
        create: (name, info) => alarmCreates.push({ name, info }),
        clear: (name) => alarmClears.push(name),
      },
      tabs: {
        sendMessage: (_tabId, message, callback) => {
          assert.equal(message.action, "getSniffedCandidates");
          callback({ urls: sniffedCandidates });
        },
      },
      cookies: { getAll: async () => cookieJar },
      // onDeterminingFilename wajib ada: background.js mendaftarkan listener
      // di top level (baris ~664), jadi tanpa mock ini seluruh service script
      // gagal dimuat dan SEMUA test badge error sebelum sempat berjalan.
      // API ini Chrome-only — extension memang hanya menarget Chromium
      // (manifest MV3 + `key`, setup-browser.sh: chrome/brave/edge).
      downloads: { onCreated: event, onDeterminingFilename: event },
      contextMenus: { onClicked: contextMenuEvent },
      action: {
        setBadgeText: ({ text }) => badges.push(text),
        setBadgeBackgroundColor: () => {},
      },
    },
  });
  vm.runInContext(source("background.js"), context);
  return {
    badges,
    requests,
    logs,
    onMessage,
    onContextMenu,
    onAlarm,
    alarmCreates,
    alarmClears,
    context,
  };
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

test("badge cleanup survives service-worker suspension via chrome alarm", async () => {
  const b = background();
  const response = new Promise((resolve) =>
    b.onMessage(
      { action: "download", url: "https://example.com/a.zip" },
      {},
      resolve,
    ),
  );
  await new Promise(setImmediate);
  b.requests[0].callback({ success: true });
  assert.equal((await response).success, true);
  assert.equal(
    b.alarmCreates.at(-1).name,
    "fastdm-badge-clear",
    "result badge has a persistent cleanup alarm",
  );
  b.onAlarm({ name: "fastdm-badge-clear" });
  assert.equal(b.badges.at(-1), "");
});

test("context-menu blob video uses the latest sniffer candidate", async () => {
  const candidate = "https://cdn.example/video.m3u8?session=abc";
  const b = background([], [candidate]);
  const pending = b.onContextMenu(
    {
      menuItemId: "fastdm-download-video",
      srcUrl: "blob:https://example.com/opaque-id",
      pageUrl: "https://example.com/watch",
    },
    { id: 7 },
  );
  await new Promise(setImmediate);
  assert.equal(b.requests.length, 1);
  assert.equal(b.requests[0].message.url, candidate);
  assert.equal(
    b.requests[0].message.headers.Referer,
    "https://example.com/watch",
  );
  b.requests[0].callback({ success: true });
  await pending;
});

test("background forwards cookie scope metadata instead of flattening values", async () => {
  const cookie = {
    name: "SID",
    value: "secret",
    domain: "example.com",
    path: "/private",
    secure: true,
    hostOnly: true,
    expirationDate: 4_000_000_000,
  };
  const b = background([cookie]);
  const response = new Promise((resolve) =>
    b.onMessage(
      { action: "download", url: "https://example.com/private/file.zip" },
      {},
      resolve,
    ),
  );
  await new Promise(setImmediate);
  assert.deepEqual(b.requests[0].message.cookies, [cookie]);
  assert.equal(b.requests[0].message.domain, "example.com");
  b.requests[0].callback({ success: true });
  assert.equal((await response).success, true);
});
