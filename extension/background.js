const NATIVE_HOST_NAME = "com.fastdm.native";

const DEFAULT_CONFIG = {
  enabled: true,
  interceptDownloads: true,
  interceptMinSize: 1048576,
  videoExtensions: [
    ".mp4",
    ".mkv",
    ".webm",
    ".avi",
    ".mov",
    ".flv",
    ".wmv",
    ".m4v",
    ".3gp",
    ".ts",
    ".m3u8",
    ".mpd",
  ],
  fileExtensions: [
    ".zip",
    ".rar",
    ".7z",
    ".tar",
    ".gz",
    ".bz2",
    ".iso",
    ".dmg",
    ".exe",
    ".msi",
    ".deb",
    ".rpm",
    ".pdf",
    ".doc",
    ".docx",
    ".xls",
    ".xlsx",
    ".mp3",
    ".flac",
    ".ogg",
    ".m4a",
    ".wav",
  ],
  excludePatterns: [],
};

let config = { ...DEFAULT_CONFIG };

// B4e: config pindah dari storage.sync ke storage.local. Config ini
// per-mesin (intersep, ambang ukuran, daftar ekstensi file) — bukan preferensi
// yang perlu ikut ke perangkat lain, sedangkan `sync` punya kuota ketat
// (8 KB/item, 512 tulis/hari, throttle) yang bisa membuat `setConfig` gagal
// tanpa terlihat oleh user (L6).
chrome.storage.local.get("config", (result) => {
  if (result.config) {
    config = { ...DEFAULT_CONFIG, ...result.config };
    return;
  }
  // Migrasi satu arah dari ≤2.9.3. Bila tulis ke local gagal, salinan di sync
  // dibiarkan utuh supaya config lama tidak hilang.
  chrome.storage.sync.get("config", (old) => {
    if (!old.config) return;
    config = { ...DEFAULT_CONFIG, ...old.config };
    chrome.storage.local.set({ config }, () => {
      if (chrome.runtime.lastError) return;
      chrome.storage.sync.remove("config");
    });
  });
});
chrome.storage.onChanged.addListener((changes, area) => {
  // `registered_id` juga hidup di local — batasi ke perubahan config saja.
  if (area === "local" && changes.config)
    config = { ...DEFAULT_CONFIG, ...changes.config.newValue };
});

// ═══════════════════════════════════════════════
// Auto-Register Extension ID
// ═══════════════════════════════════════════════

/**
 * Kirim Extension ID ke native host saat pertama kali jalan.
 * Native host akan otomatis update manifest.
 */
function registerExtensionId() {
  const extId = chrome.runtime.id;
  if (!extId) return;

  chrome.storage.local.get("registered_id", (result) => {
    // Skip jika sudah pernah register dengan ID yang sama
    if (result.registered_id === extId) {
      return;
    }

    sendToNative({
      action: "register",
      extension_id: extId,
    })
      .then((response) => {
        if (response && response.success) {
          chrome.storage.local.set({ registered_id: extId });
          console.log("[FastDM] Extension registered:", extId);
        }
      })
      .catch(() => {
        // Silent fail — akan retry saat message berikutnya
        // Tidak perlu spam console
      });
  });
}

// Register saat extension di-load
registerExtensionId();

// Register saat pertama kali install
chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === "install" || details.reason === "update") {
    // Reset flag agar register ulang
    chrome.storage.local.remove("registered_id", () => {
      registerExtensionId();
    });
  }

  // Context menus — removeAll dulu, kalau tidak `create` dengan id yang sama
  // error (mis. saat update extension) dan menu tidak pernah terbuat.
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "fastdm-download-link",
      title: "⚡ Unduh dengan Fast DM",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-video",
      title: "⚡ Unduh Video dengan Fast DM",
      contexts: ["video", "audio"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-image",
      title: "⚡ Unduh Gambar dengan Fast DM",
      contexts: ["image"],
    });
  });
});

// Register ulang saat browser restart (deduplikasi internal:
// registerExtensionId cek storage & di-skip bila ID sudah terdaftar)
chrome.runtime.onStartup.addListener(() => {
  registerExtensionId();
});

// ═══════════════════════════════════════════════
// Native Messaging
// ═══════════════════════════════════════════════

// v2.3.0 (L7): batas waktu eksplisit. Native host bisa menahan request cukup
// lama saat cold start GUI (poll socket hingga ±15 dtk + forward 5 dtk);
// tanpa timeout, promise menggantung tanpa umpan balik ke user.
const NATIVE_TIMEOUT_MS = 25_000;

function sendToNative(message) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let timer;
    const finish = (fn, arg) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      fn(arg);
    };
    timer = setTimeout(
      () =>
        finish(
          reject,
          new Error("Timeout — Fast DM tidak merespons (apakah baru mulai?)"),
        ),
      NATIVE_TIMEOUT_MS,
    );
    try {
      chrome.runtime.sendNativeMessage(NATIVE_HOST_NAME, message, (resp) => {
        if (chrome.runtime.lastError) {
          finish(reject, chrome.runtime.lastError);
          return;
        }
        finish(resolve, resp);
      });
    } catch (err) {
      finish(reject, err);
    }
  });
}

async function sendDownload(
  url,
  filename = null,
  headers = {},
  quality = null,
  cookies = null,
  domain = null,
) {
  if (!filename) {
    try {
      const urlObj = new URL(url);
      const path = decodeURIComponent(urlObj.pathname);
      const parts = path.split("/").filter(Boolean);
      if (parts.length > 0) {
        const last = parts[parts.length - 1];
        if (last.includes(".")) filename = last;
      }
    } catch (e) {
      /* ignore */
    }
  }

  const message = {
    action: "download",
    url: url,
    filename: filename,
    headers: headers,
    extension_id: chrome.runtime.id,
  };

  // Tambah quality jika ada (untuk YouTube)
  if (quality) {
    message.quality = quality;
  }

  // Ambil cookies situs via API bila tidak dikirim eksplisit
  // (download login-protected — dipakai yt-dlp & aria2 --load-cookies)
  if (!cookies) {
    try {
      const jar = await chrome.cookies.getAll({ url });
      if (jar && jar.length > 0) {
        cookies = jar.map((c) => c.name + "=" + c.value).join("; ");
        domain = new URL(url).hostname;
      }
    } catch (e) {
      /* ignore */
    }
  }

  // Cookies halaman (untuk yt-dlp — video membersih+/login)
  if (cookies && domain) {
    message.cookies = cookies;
    message.domain = domain;
  }

  // B4f: badge pending — sendToNative menunggu hingga 25 detik sebelum
  // timeout. Tanpa penanda ini user tidak dapat umpan balik sama sekali saat
  // native host lambat atau tidak merespons. `holdMs = 0` menahannya sampai
  // badge hasil (⬇/!) menggantikannya.
  showBadge("…", "#a6adc8", 0);

  try {
    const response = await sendToNative(message);
    if (!response || !response.success) {
      showBadge("!", "#f38ba8");
      return (
        response || {
          success: false,
          error: "Native host tidak memberi respons",
        }
      );
    }
    // URL bisa mengandung token; cukup log status, bukan URL/nama file.
    console.log("[FastDM] Download accepted");
    showBadge("⬇", "#89b4fa");
    return response;
  } catch (err) {
    console.error("[FastDM] Failed:", err);
    showBadge("!", "#f38ba8");
    return null;
  }
}

// B4f: timer badge dilacak. Tanpa ini, urutan "…" → "⬇" (atau dua unduhan
// berdekatan) membuat timeout milik badge LAMA menghapus badge yang baru
// dipasang sebelum 3 detik habis.
let badgeTimer = null;

function showBadge(text, color, holdMs = 3000) {
  if (badgeTimer) clearTimeout(badgeTimer);
  chrome.action.setBadgeText({ text });
  chrome.action.setBadgeBackgroundColor({ color });
  badgeTimer =
    holdMs > 0
      ? setTimeout(() => {
          badgeTimer = null;
          chrome.action.setBadgeText({ text: "" });
        }, holdMs)
      : null;
}

// ═══════════════════════════════════════════════
// Download Interception
// ═══════════════════════════════════════════════

// URL yang di-restart ulang oleh fallback kita sendiri. Tanpa ini, download
// fallback akan ter-intercept lagi → cancel → fallback lagi → loop tak
// berujung saat native host tidak tersedia.
//
// v2.9.3: entri kedaluwarsa otomatis. Dulu entri hanya dihapus saat event
// onCreated untuk URL yang sama tiba — kalau download fallback tidak pernah
// terbentuk (user menutup dialog "Simpan sebagai", URL ditolak Chrome, dst.)
// entri tertinggal selamanya dan menumpuk di service worker; unduhan ULANG
// URL yang sama juga jadi ikut dilewatkan (tidak dikirim ke Fast DM).
const SELF_INITIATED_TTL_MS = 60_000;
const selfInitiated = new Map();

function markSelfInitiated(url) {
  selfInitiated.set(url, Date.now() + SELF_INITIATED_TTL_MS);
}

/** true = event ini berasal dari fallback kita sendiri (sekali pakai). */
function consumeSelfInitiated(url) {
  const now = Date.now();
  // Sapu entri kedaluwarsa sekalian — jumlahnya kecil, biayanya sepele.
  for (const [key, expiry] of selfInitiated) {
    if (expiry <= now) selfInitiated.delete(key);
  }
  if (!selfInitiated.has(url)) return false;
  selfInitiated.delete(url);
  return true;
}

/** true = fallback kita sendiri, TANPA menghapus (peek). Dipakai
 * onDeterminingFilename agar tidak bersaing hapus dengan onCreated — urutan
 * kedua event bisa berbeda-beda per unduhan. */
function isSelfInitiated(url) {
  const now = Date.now();
  for (const [key, expiry] of selfInitiated) {
    if (expiry <= now) selfInitiated.delete(key);
  }
  return selfInitiated.has(url);
}

// URL yang SUDAH diputuskan (di-intercept ke native host ATAU dibiarkan jalan
// di Chrome karena fallback). Dipakai onDeterminingFilename & onCreated agar
// satu unduhan tidak diproses dua kali — dua event itu bisa tumpang tindih.
const handledUrls = new Map();

function markHandled(url) {
  handledUrls.set(url, Date.now() + SELF_INITIATED_TTL_MS);
}

function wasHandled(url) {
  const now = Date.now();
  for (const [key, expiry] of handledUrls) {
    if (expiry <= now) handledUrls.delete(key);
  }
  return handledUrls.has(url);
}

/// Nama file terakhir dari path (dipakai onCreated & onDeterminingFilename).
function basename(path) {
  if (!path) return null;
  const parts = path.replace(/\\/g, "/").split("/");
  const last = parts[parts.length - 1];
  return last && last.includes(".") ? last : null;
}

/// Apakah nama file (hasil resolve Chrome dari Content-Disposition) berakhiran
/// ekstensi yang masuk daftar intersep?
function filenameHasInterceptedExt(filename) {
  if (!filename) return false;
  const lower = filename.toLowerCase();
  const allExts = [...config.videoExtensions, ...config.fileExtensions];
  return allExts.some((ext) => lower.endsWith(ext));
}

chrome.downloads.onCreated.addListener(async (downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  // Jangan intercept download yang kita sendiri buat ulang (fallback) —
  // biarkan jalan di Chrome; tandai handled agar onDeterminingFilename skip.
  if (consumeSelfInitiated(url)) {
    markHandled(url);
    return;
  }

  // Sudah diputuskan onDeterminingFilename (bila menyala lebih dulu)?
  if (wasHandled(url)) return;

  // CATATAN (B18): saat onCreated, fileSize umumnya masih 0 dan mime kosong,
  // jadi deteksi dalam praktiknya mengandalkan ekstensi file di URL
  // (limitasi API chrome.downloads — bukan bug). Kasus URL tanpa ekstensi
  // (query-string download) ditangani onDeterminingFilename di bawah.
  if (!shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime))
    return;

  // Tandai SEBELUM cancel: onDeterminingFilename untuk unduhan yang sama
  // (bila sempat menyala) harus melewatkannya — jangan intercept ganda.
  markHandled(url);

  // Cancel Chrome download immediately to prevent partial file
  chrome.downloads.cancel(downloadItem.id, () => {
    chrome.downloads.erase({ id: downloadItem.id });
  });

  const filename = basename(downloadItem.filename);

  const headers = {};
  if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

  const result = await sendDownload(url, filename, headers).catch(() => null);
  if (!result || !result.success) {
    // Fallback: restart download in Chrome normally
    // (omit filename when unknown — Chrome rejects null for optional string args)
    const opts = { url, saveAs: true };
    if (filename) opts.filename = filename;
    markSelfInitiated(url);
    chrome.downloads.download(opts);
  }
});

// v2.10.5: jaring kedua untuk unduhan yang di onCreated belum jelas — URL
// tanpa ekstensi (mis. .../download?id=123 yang mengembalikan .zip) di mana
// fileSize/mime masih kosong di onCreated. onDeterminingFilename dipanggil
// SETELAH Chrome menyelesaikan nama file (dari Content-Disposition), sehingga
// ekstensi final (dan kadang mime/ukuran) sudah tersedia.
chrome.downloads.onDeterminingFilename.addListener((downloadItem, suggest) => {
  const keep = () => suggest({ filename: downloadItem.filename });

  if (!config.enabled || !config.interceptDownloads) return keep();

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:"))
    return keep();

  // Fallback kita sendiri / sudah diputuskan onCreated → biarkan Chrome lanjut.
  if (isSelfInitiated(url) || wasHandled(url)) return keep();

  const filename = basename(downloadItem.filename);
  const intercept =
    shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime) ||
    filenameHasInterceptedExt(filename);

  if (!intercept) return keep();

  markHandled(url);
  // Cancel + teruskan ke native host. keep() tetap dipanggil supaya bila
  // cancel gagal/racing, unduhan lanjut dengan nama final yang benar.
  chrome.downloads.cancel(downloadItem.id, () => {
    chrome.downloads.erase({ id: downloadItem.id });
  });

  const headers = {};
  if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

  sendDownload(url, filename, headers).then((result) => {
    if (!result || !result.success) {
      const opts = { url, saveAs: true };
      if (filename) opts.filename = filename;
      markSelfInitiated(url);
      chrome.downloads.download(opts);
    }
  });

  keep();
});

function shouldInterceptUrl(url, fileSize, mimeType) {
  const urlLower = url.toLowerCase();

  for (const pattern of config.excludePatterns) {
    if (urlLower.includes(pattern)) return false;
  }

  // JANGAN intercept YouTube — biarkan Fast DM GUI handle
  try {
    const urlObj = new URL(url);
    const ytHosts = [
      "youtube.com",
      "www.youtube.com",
      "youtu.be",
      "m.youtube.com",
      "music.youtube.com",
    ];
    if (ytHosts.includes(urlObj.hostname)) {
      return false;
    }
  } catch (e) {
    /* ignore */
  }

  try {
    const path = new URL(url).pathname.toLowerCase();
    const allExts = [...config.videoExtensions, ...config.fileExtensions];
    for (const ext of allExts) {
      if (path.endsWith(ext)) return true;
    }
  } catch (e) {
    /* ignore */
  }

  if (mimeType) {
    const interceptMimes = [
      "video/",
      "audio/",
      "application/zip",
      "application/x-rar",
      "application/x-7z",
      "application/gzip",
      "application/pdf",
      "application/x-iso9660-image",
      "application/x-bzip2",
      "application/x-tar",
    ];
    for (const mime of interceptMimes) {
      if (mimeType.startsWith(mime)) return true;
    }
  }

  if (fileSize && fileSize > config.interceptMinSize) return true;

  return false;
}

// ═══════════════════════════════════════════════
// Context Menu
// ═══════════════════════════════════════════════

chrome.contextMenus.onClicked.addListener((info, tab) => {
  let url = null;
  let filename = null;

  switch (info.menuItemId) {
    case "fastdm-download-link":
      url = info.linkUrl;
      break;
    case "fastdm-download-video":
      url = info.srcUrl;
      break;
    case "fastdm-download-image":
      url = info.srcUrl;
      break;
  }

  if (!url) return;

  const headers = {};
  if (info.pageUrl) headers["Referer"] = info.pageUrl;

  try {
    const path = new URL(url).pathname;
    const decoded = decodeURIComponent(path);
    const parts = decoded.split("/").filter(Boolean);
    if (parts.length > 0) {
      const last = parts[parts.length - 1];
      if (last && last.includes(".")) filename = last;
    }
  } catch (e) {
    /* ignore */
  }

  sendDownload(url, filename, headers);
});

// ═══════════════════════════════════════════════
// Messages
// ═══════════════════════════════════════════════

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === "download") {
    sendDownload(
      message.url,
      message.filename,
      message.headers || {},
      message.quality || null,
      message.cookies || null,
      message.domain || null,
    )
      // B14: sendDownload tidak pernah me-reject (internal catch → null),
      // jadi terjemahkan null jadi respons gagal yang eksplisit.
      .then((r) =>
        sendResponse(
          r || { success: false, error: "Gagal mengirim ke native host" },
        ),
      )
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getConfig") {
    sendResponse(config);
    return false;
  }
  if (message.action === "setConfig") {
    config = { ...config, ...message.config };
    chrome.storage.local.set({ config });
    sendResponse({ success: true });
    return false;
  }
  if (message.action === "ping") {
    sendToNative({ action: "ping" })
      .then((r) => sendResponse(r))
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getStatus") {
    sendToNative({ action: "list" })
      .then((r) => sendResponse(r))
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getExtensionId") {
    sendResponse({ id: chrome.runtime.id });
    return false;
  }
});
