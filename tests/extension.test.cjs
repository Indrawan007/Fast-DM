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

// Event mock yang menangkap handler untuk bisa di-fire oleh test.
function listener() {
  let handler = null;
  return {
    addListener(h) {
      handler = h;
    },
    fire(...args) {
      return handler ? handler(...args) : undefined;
    },
  };
}

function background() {
  const badges = [];
  const requests = [];
  const logs = [];
  const cancels = [];
  const fallbacks = [];
  let onMessage;
  const event = { addListener() {} };
  const downloads = {
    onCreated: listener(),
    onDeterminingFilename: listener(),
    cancel: (id, callback) => {
      cancels.push(id);
      if (callback) callback();
    },
    erase: () => {},
    download: (opts) => {
      fallbacks.push(opts);
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
      cookies: { getAll: async () => [] },
      downloads,
      contextMenus: { onClicked: event },
      action: {
        setBadgeText: ({ text }) => badges.push(text),
        setBadgeBackgroundColor: () => {},
      },
    },
  });
  vm.runInContext(source("background.js"), context);
  return { badges, requests, logs, onMessage, context, downloads, cancels, fallbacks };
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

// ── v2.10.5: onDeterminingFilename (jaring kedua untuk URL query-string) ──

test("onDeterminingFilename intercepts query-string downloads by resolved filename", async () => {
  const b = background();
  const suggested = [];
  b.downloads.onDeterminingFilename.fire(
    {
      id: 7,
      url: "https://example.com/download?id=123",
      finalUrl: "https://example.com/download?id=123",
      filename: "/home/user/Downloads/report.zip",
      referrer: "https://example.com/page",
      fileSize: 0,
      mime: "",
    },
    (o) => suggested.push(o),
  );
  await new Promise(setImmediate);
  assert.equal(b.cancels.length, 1, "query-string download harus di-cancel");
  assert.equal(b.cancels[0], 7);
  assert.equal(b.requests.length, 1, "diteruskan ke native host");
  assert.equal(b.requests[0].message.filename, "report.zip");
  assert.equal(b.requests[0].message.headers.Referer, "https://example.com/page");
  assert.equal(suggested.length, 1, "suggest tetap dipanggil (anti-race)");
  assert.equal(suggested[0].filename, "/home/user/Downloads/report.zip");
});

test("onDeterminingFilename leaves non-media downloads untouched", async () => {
  const b = background();
  const suggested = [];
  b.downloads.onDeterminingFilename.fire(
    {
      id: 8,
      url: "https://example.com/notes",
      finalUrl: "https://example.com/notes",
      filename: "/home/user/Downloads/notes.txt",
      referrer: "",
      fileSize: 0,
      mime: "",
    },
    (o) => suggested.push(o),
  );
  await new Promise(setImmediate);
  assert.equal(b.cancels.length, 0);
  assert.equal(b.requests.length, 0);
  assert.equal(suggested.length, 1);
});

test("onDeterminingFilename does not double-handle a URL onCreated already intercepted", async () => {
  const b = background();
  b.downloads.onCreated.fire({
    id: 9,
    url: "https://example.com/movie.mp4",
    finalUrl: "https://example.com/movie.mp4",
    filename: "/home/user/Downloads/movie.mp4",
    referrer: "",
    fileSize: 0,
    mime: "",
  });
  await new Promise(setImmediate);
  assert.equal(b.cancels.length, 1, "onCreated harus intercept movie.mp4");

  const suggested = [];
  b.downloads.onDeterminingFilename.fire(
    {
      id: 9,
      url: "https://example.com/movie.mp4",
      finalUrl: "https://example.com/movie.mp4",
      filename: "/home/user/Downloads/movie.mp4",
      referrer: "",
      fileSize: 0,
      mime: "",
    },
    (o) => suggested.push(o),
  );
  await new Promise(setImmediate);
  assert.equal(b.cancels.length, 1, "tidak boleh cancel dua kali");
  assert.equal(suggested.length, 1);
});

test("fallback download is not re-intercepted (no loop)", async () => {
  const b = background();
  b.downloads.onCreated.fire({
    id: 10,
    url: "https://example.com/movie.mp4",
    finalUrl: "https://example.com/movie.mp4",
    filename: "/home/user/Downloads/movie.mp4",
    referrer: "",
    fileSize: 0,
    mime: "",
  });
  await new Promise(setImmediate);
  assert.equal(b.requests.length, 1);
  b.requests[0].callback({ success: false, error: "host down" }); // native gagal
  await new Promise(setImmediate);
  assert.equal(b.fallbacks.length, 1, "fallback ke Chrome di-issue");

  // Unduhan fallback yang sama kini memicu onDeterminingFilename → harus dilewati.
  const suggested = [];
  b.downloads.onDeterminingFilename.fire(
    {
      id: 11,
      url: "https://example.com/movie.mp4",
      finalUrl: "https://example.com/movie.mp4",
      filename: "/home/user/Downloads/movie.mp4",
      referrer: "",
      fileSize: 0,
      mime: "",
    },
    (o) => suggested.push(o),
  );
  await new Promise(setImmediate);
  assert.equal(b.cancels.length, 1, "fallback tidak boleh di-cancel lagi");
  assert.equal(b.requests.length, 1, "tidak ada request native ekstra");
  assert.equal(suggested.length, 1);
});
