// Dependency-free browser API mocks. Run: node --test tests/extension.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const vm = require("node:vm");

function source(file) {
  return readFileSync(join(__dirname, "..", "extension", file), "utf8");
}

function popup(pingResponse = { success: true }) {
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
      else if (message.action === "ping") callback(pingResponse);
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
    status: elements.get("status-dot"),
    statusText: elements.get("status-text"),
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

// v3.3.3: native host TIDAK lagi menyalakan Fast-DM untuk `ping` — popup yang
// dibuka saat aplikasi mati kini benar-benar menerima `success: false` dan
// harus menampilkannya, bukan "Terhubung ✓" hasil cold start tak diminta.
test("popup reports Fast DM as not running when ping fails", () => {
  const p = popup({ success: false, error: "Fast DM tidak berjalan (socket)" });
  assert.equal(p.status.className, "status-dot disconnected");
  assert.equal(
    p.statusText.textContent,
    "Fast DM tidak berjalan. Jalankan: fast-dm",
  );
});

function background(cookieJar = [], sniffedCandidates = []) {
  const badges = [];
  const requests = [];
  const logs = [];
  const alarmCreates = [];
  const alarmClears = [];
  let onMessage;
  let onContextMenu;
  let onAlarm;
  let onDownloadsCreated;
  let onDownloadsChanged;
  const cancelled = [];
  const erased = [];
  const downloaded = [];
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
    navigator: { userAgent: "FastDM-Test-Browser/150.0" },
    setTimeout: () => 1,
    clearTimeout: () => {},
    console: {
      log: (...args) => logs.push(args.join(" ")),
      error() {},
      debug() {},
    },
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
      // v3.2.3 (A6): `onChanged` juga terdaftar di top level sekarang (jalur
      // erase-on-interrupted), dan `cancel`/`erase`/`download` direkam agar
      // urutan cancel→interrupted→erase bisa diuji.
      downloads: {
        onCreated: {
          addListener(handler) {
            onDownloadsCreated = handler;
          },
        },
        onDeterminingFilename: event,
        onChanged: {
          addListener(handler) {
            onDownloadsChanged = handler;
          },
        },
        cancel: (id, cb) => {
          cancelled.push(id);
          cb?.();
        },
        erase: (opts, cb) => {
          erased.push(opts.id);
          cb?.();
        },
        download: (opts, cb) => {
          downloaded.push(opts);
          cb?.();
        },
      },
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
    onDownloadsCreated,
    onDownloadsChanged,
    cancelled,
    erased,
    downloaded,
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

test("context-menu link and image items still download (videoMenu guard)", async () => {
  // Regresi v3.2.2: `videoMenu` yang tidak terdefinisi membuat handler async
  // melempar ReferenceError untuk SEMUA item menu, jadi link/gambar biasa pun
  // tidak pernah terkirim. Test ini mengunci jalur non-video tetap hidup dan
  // TIDAK memanggil sniffer (hanya menu video ber-URL blob: yang boleh).
  for (const item of [
    {
      menuItemId: "fastdm-download-link",
      linkUrl: "https://example.com/files/report.pdf",
      pageUrl: "https://example.com/page",
      expectUrl: "https://example.com/files/report.pdf",
      expectName: "report.pdf",
    },
    {
      menuItemId: "fastdm-download-image",
      srcUrl: "https://cdn.example/pics/photo.jpg",
      pageUrl: "https://example.com/gallery",
      expectUrl: "https://cdn.example/pics/photo.jpg",
      expectName: "photo.jpg",
    },
  ]) {
    const b = background();
    const pending = b.onContextMenu(item, { id: 7 });
    await new Promise(setImmediate);
    assert.equal(b.requests.length, 1, `${item.menuItemId} mengirim unduhan`);
    assert.equal(b.requests[0].message.url, item.expectUrl);
    assert.equal(b.requests[0].message.filename, item.expectName);
    assert.equal(b.requests[0].message.headers.Referer, item.pageUrl);
    b.requests[0].callback({ success: true });
    await pending;
    assert.equal(b.badges.at(-1), "⬇");
  }
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

// v3.2.3 (A6): unduhan yang di-intercept dibatalkan segera, tetapi `erase`
// ditunda sampai Chrome melaporkan `interrupted` — memanggil erase di dalam
// callback cancel ditolak Chrome karena item masih `in_progress`, sehingga
// entri "dibatalkan" tertinggal di shelf unduhan browser.
test("intercepted download is erased only after it reports interrupted", async () => {
  const b = background();
  const flushed = () => new Promise(setImmediate);
  assert.equal(typeof b.onDownloadsCreated, "function");
  assert.equal(typeof b.onDownloadsChanged, "function");

  // Handler `onCreated` bersifat async dan menunggu balasan native host
  // (mock `setTimeout` di VM tidak menjadwalkan apa pun), jadi JANGAN di-await
  // di sini — panggil lalu flush microtask, sama seperti test lain di suite ini.
  // `state: "in_progress"` = unduhan BARU (bukan entri riwayat) — lihat test
  // "history entries replayed by onCreated" untuk kebalikannya.
  void b.onDownloadsCreated({
    id: 42,
    url: "https://example.com/movie.mp4",
    finalUrl: "https://example.com/movie.mp4",
    filename: "/home/user/Downloads/movie.mp4",
    fileSize: 10_485_760,
    mime: "video/mp4",
    referrer: "https://example.com/watch",
    state: "in_progress",
  });
  await flushed();

  assert.deepEqual(b.cancelled, [42], "cancel dipanggil saat intersep");
  assert.equal(
    b.erased.length,
    0,
    "erase TIDAK boleh langsung (item in_progress)",
  );
  assert.equal(b.requests.length, 1, "unduhan dikirim ke native host");
  assert.equal(
    b.requests[0].message.headers["User-Agent"],
    "FastDM-Test-Browser/150.0",
  );
  assert.equal(
    b.requests[0].message.headers.Referer,
    "https://example.com/watch",
  );

  // Delta yang tidak relevan (masih berjalan) tidak boleh memicu erase.
  b.onDownloadsChanged({ id: 42, state: { current: "in_progress" } });
  assert.equal(b.erased.length, 0, "delta in_progress diabaikan");

  b.onDownloadsChanged({ id: 42, state: { current: "interrupted" } });
  assert.deepEqual(b.erased, [42], "erase setelah interrupted");

  // Delta berikutnya untuk id yang sama tidak menghapus ulang.
  b.onDownloadsChanged({ id: 42, state: { current: "interrupted" } });
  assert.deepEqual(b.erased, [42], "erase hanya sekali per unduhan");

  // Unduhan yang TIDAK kita batalkan tidak boleh ikut dihapus dari shelf.
  b.onDownloadsChanged({ id: 99, state: { current: "interrupted" } });
  assert.deepEqual(b.erased, [42], "unduhan lain tidak disentuh");

  // Unduhan yang berakhir `complete` (bukan `interrupted`) tidak di-erase,
  // tetapi entri pelacaknya tetap dilepas agar tidak menumpuk.
  void b.onDownloadsCreated({
    id: 43,
    url: "https://example.com/other.mp4",
    finalUrl: "https://example.com/other.mp4",
    filename: "/home/user/Downloads/other.mp4",
    fileSize: 10_485_760,
    mime: "video/mp4",
    state: "in_progress",
  });
  await flushed();
  assert.deepEqual(b.cancelled, [42, 43]);
  b.onDownloadsChanged({ id: 43, state: { current: "complete" } });
  assert.deepEqual(b.erased, [42], "complete tidak ikut di-erase");
  b.onDownloadsChanged({ id: 43, state: { current: "interrupted" } });
  assert.deepEqual(b.erased, [42], "entri sudah dilepas → tidak di-erase lagi");

  b.requests[0].callback({ success: true });
  await flushed();
  assert.equal(b.badges.at(-1), "⬇");
});

// v3.2.3 (A7): jalur fallback memakai `saveAs: true` TANPA `filename` — Chrome
// mengabaikan `filename` saat dialog "Simpan sebagai" terbuka, jadi mengirim
// keduanya hanya menyesatkan pembaca kode.
test("fallback download uses saveAs without a conflicting filename", async () => {
  const b = background();
  void b.onDownloadsCreated({
    id: 7,
    url: "https://example.com/big.iso",
    finalUrl: "https://example.com/big.iso",
    filename: "/home/user/Downloads/big.iso",
    fileSize: 10_485_760,
    mime: "application/x-iso9660-image",
    referrer: "https://example.com/",
    state: "in_progress",
  });
  await new Promise(setImmediate);

  assert.equal(b.requests.length, 1);
  b.requests[0].callback({ success: false, error: "Ditolak" });
  await new Promise(setImmediate);

  assert.equal(b.downloaded.length, 1, "fallback memulai ulang unduhan");
  // Objek dibuat di dalam konteks VM (prototipe berbeda), jadi bandingkan per
  // field — `deepStrictEqual` menolak meskipun isinya identik.
  assert.equal(b.downloaded[0].url, "https://example.com/big.iso");
  assert.equal(b.downloaded[0].saveAs, true);
  assert.equal(
    "filename" in b.downloaded[0],
    false,
    "tanpa field filename yang akan diabaikan Chrome",
  );
  assert.deepEqual(Object.keys(b.downloaded[0]).sort(), ["saveAs", "url"]);
});

// v3.2.7: saat browser start, Chrome memuat riwayat unduhan dari disk dan
// memancarkan `downloads.onCreated` untuk SETIAP entri lama (state `complete`
// / `interrupted`) — bukan hanya unduhan baru. Tanpa pemeriksaan `state`,
// handler intersep memperlakukan entri riwayat itu sebagai unduhan baru:
// `sendNativeMessage` → native host menyalakan GUI → URL lama diunduh ulang
// pada setiap start browser. Entri yang tertinggal berasal dari v3.2.2 (erase
// di dalam callback cancel ditolak Chrome → entri "dibatalkan" tetap di
// riwayat) dan dari jalur fallback `saveAs`.
test("history entries replayed by onCreated at startup are not re-sent to Fast DM", async () => {
  const b = background();
  const flushed = () => new Promise(setImmediate);

  // Entri v3.2.2: di-intercept + cancel, tetapi erase gagal → tertinggal.
  void b.onDownloadsCreated({
    id: 501,
    url: "https://files.example/archive-01.zip",
    finalUrl: "https://cdn.example/dl/archive-01.zip?token=old",
    filename: "/home/user/Downloads/archive-01.zip",
    fileSize: 52_428_800,
    mime: "application/zip",
    referrer: "https://files.example/folder",
    state: "interrupted",
    error: "USER_CANCELED",
    exists: false,
  });
  // Entri fallback `saveAs` yang dulu selesai lewat Chrome.
  void b.onDownloadsCreated({
    id: 502,
    url: "https://files.example/archive-02.zip",
    finalUrl: "https://files.example/archive-02.zip",
    filename: "/home/user/Downloads/archive-02.zip",
    fileSize: 52_428_800,
    mime: "application/zip",
    state: "complete",
    exists: true,
  });
  await flushed();

  assert.equal(
    b.requests.length,
    0,
    "entri riwayat TIDAK boleh dikirim ke native host (memicu GUI + unduh ulang)",
  );
  assert.deepEqual(b.cancelled, [], "tidak ada cancel untuk entri riwayat");
  assert.deepEqual(b.downloaded, [], "tidak ada fallback untuk entri riwayat");
  assert.deepEqual(b.badges, [], "tidak ada badge pending saat startup");

  // Unduhan BARU untuk URL yang sama tetap di-intercept — pemeriksaan state
  // tidak boleh mematikan intersepsi normal.
  void b.onDownloadsCreated({
    id: 503,
    url: "https://files.example/archive-01.zip",
    finalUrl: "https://cdn.example/dl/archive-01.zip?token=new",
    filename: "/home/user/Downloads/archive-01.zip",
    fileSize: 52_428_800,
    mime: "application/zip",
    referrer: "https://files.example/folder",
    state: "in_progress",
    exists: false,
  });
  await flushed();

  assert.deepEqual(
    b.cancelled,
    [503],
    "unduhan baru tetap dibatalkan di Chrome",
  );
  assert.equal(b.requests.length, 1, "unduhan baru tetap dikirim ke Fast DM");
  assert.equal(
    b.requests[0].message.url,
    "https://cdn.example/dl/archive-01.zip?token=new",
  );
  b.requests[0].callback({ success: true });
  await flushed();
  assert.equal(b.badges.at(-1), "⬇");
});

for (const variant of ["browser", "explicit", "no-navigator"]) {
  test(`download forwards User-Agent safely: ${variant}`, async () => {
    const b = background();
    const headers = { Referer: "https://example.com/post/1" };
    if (variant === "explicit") headers["uSeR-aGeNt"] = "ExplicitBrowser/1.0";
    if (variant === "no-navigator") delete b.context.navigator;
    const original = { ...headers };
    const response = new Promise((resolve) =>
      b.onMessage(
        { action: "download", url: "https://example.com/file.zip", headers },
        {},
        resolve,
      ),
    );
    await new Promise(setImmediate);
    const sent = b.requests[0].message.headers;
    const uaKeys = Object.keys(sent).filter(
      (key) => key.toLowerCase() === "user-agent",
    );
    assert.equal(sent.Referer, headers.Referer);
    assert.deepEqual(headers, original, "caller headers must not be mutated");
    if (variant === "no-navigator") {
      assert.equal(uaKeys.length, 0);
    } else {
      assert.equal(
        uaKeys.length,
        1,
        "must not send duplicate User-Agent headers",
      );
      assert.equal(
        sent[uaKeys[0]],
        variant === "explicit"
          ? "ExplicitBrowser/1.0"
          : "FastDM-Test-Browser/150.0",
      );
    }
    b.requests[0].callback({ success: true });
    assert.equal((await response).success, true);
  });
}

// v3.3.2: server menolak Fast-DM (HTTP 403 / halaman HTML) setelah unduhan
// diterima — extension wajib mengembalikannya ke browser, bukan membiarkan
// user kehilangan unduhan yang sudah dibatalkan di browser.
async function interceptThenPoll(states) {
  const b = background();
  const flushed = () => new Promise(setImmediate);
  // Hanya jeda polling handback yang dijalankan; timeout native (25 dtk) dan
  // timer badge tetap tidak pernah terpicu.
  b.context.setTimeout = (callback, delay) => {
    if (delay === 2000) setImmediate(callback);
    return 1;
  };
  const url = "https://files.example/4c59afe7.zip?sig=abc";
  void b.onDownloadsCreated({
    id: 7,
    url,
    finalUrl: url,
    filename: "/home/user/Downloads/4c59afe7.zip",
    fileSize: 50_000_000,
    mime: "application/zip",
    referrer: "https://files.example/page",
    state: "in_progress",
  });
  await flushed();
  assert.equal(b.requests[0].message.action, "download");
  b.requests[0].callback({ success: true, id: "dl_abc" });
  for (const state of states) {
    for (let i = 0; i < 5; i++) await flushed();
    const req = b.requests.at(-1);
    assert.equal(req.message.action, "handback");
    assert.equal(req.message.id, "dl_abc");
    req.callback({ success: true, id: "dl_abc", message: state });
  }
  for (let i = 0; i < 5; i++) await flushed();
  return { b, url };
}

test("rejected download is handed back to the browser after pending polls", async () => {
  const { b, url } = await interceptThenPoll([
    "pending",
    "pending",
    "handback",
  ]);
  assert.equal(b.requests.length, 4, "download + 3 poll, lalu berhenti");
  assert.equal(
    JSON.stringify(b.downloaded),
    JSON.stringify([{ url }]),
    "browser mengunduh ulang, tanpa saveAs",
  );
  assert.equal(b.badges.at(-1), "↩");

  // Unduhan hasil handback tidak boleh dicegat & dikirim ke Fast-DM lagi.
  b.onDownloadsCreated({
    id: 8,
    url,
    finalUrl: url,
    filename: "/home/user/Downloads/4c59afe7.zip",
    fileSize: 50_000_000,
    mime: "application/zip",
    state: "in_progress",
  });
  await new Promise(setImmediate);
  assert.equal(b.requests.length, 4);
  assert.deepEqual(b.cancelled, [7]);
});

test("download that starts flowing is never handed back", async () => {
  const { b } = await interceptThenPoll(["pending", "done"]);
  assert.equal(b.requests.length, 3, "polling berhenti pada done");
  assert.equal(b.downloaded.length, 0);
});
