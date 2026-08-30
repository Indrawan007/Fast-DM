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

chrome.storage.sync.get("config", (result) => {
  if (result.config) config = { ...DEFAULT_CONFIG, ...result.config };
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "sync" && changes.config)
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

// Register ulang saat Chrome restart
chrome.runtime.onStartup.addListener(() => {
  registerExtensionId();
});

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
      title: "⚡ Download with Fast DM",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-video",
      title: "⚡ Download Video with Fast DM",
      contexts: ["video", "audio"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-image",
      title: "⚡ Download Image with Fast DM",
      contexts: ["image"],
    });
  });
});

chrome.runtime.onStartup.addListener(() => {
  chrome.storage.local.get("registered_id", (result) => {
    if (!result.registered_id) {
      registerExtensionId();
    }
  });
});

// ═══════════════════════════════════════════════
// Native Messaging
// ═══════════════════════════════════════════════

function sendToNative(message) {
  return new Promise((resolve, reject) => {
    try {
      chrome.runtime.sendNativeMessage(NATIVE_HOST_NAME, message, (resp) => {
        if (chrome.runtime.lastError) {
          reject(chrome.runtime.lastError);
          return;
        }
        resolve(resp);
      });
    } catch (err) {
      reject(err);
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

  try {
    const response = await sendToNative(message);
    console.log("[FastDM] Download sent:", filename || url);
    showBadge("⬇", "#89b4fa");
    return response;
  } catch (err) {
    console.error("[FastDM] Failed:", err);
    showBadge("!", "#f38ba8");
    return null;
  }
}

function showBadge(text, color) {
  chrome.action.setBadgeText({ text });
  chrome.action.setBadgeBackgroundColor({ color });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 3000);
}

// ═══════════════════════════════════════════════
// Download Interception
// ═══════════════════════════════════════════════

// URL yang di-restart ulang oleh fallback kita sendiri. Tanpa ini, download
// fallback akan ter-intercept lagi → cancel → fallback lagi → loop tak
// berujung saat native host tidak tersedia.
const selfInitiated = new Set();

chrome.downloads.onCreated.addListener(async (downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  // Jangan intercept download yang kita sendiri buat ulang (fallback)
  if (selfInitiated.has(url)) {
    selfInitiated.delete(url);
    return;
  }

  if (!shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime))
    return;

  // Cancel Chrome download immediately to prevent partial file
  chrome.downloads.cancel(downloadItem.id, () => {
    chrome.downloads.erase({ id: downloadItem.id });
  });

  let filename = null;
  if (downloadItem.filename) {
    const parts = downloadItem.filename.replace(/\\/g, "/").split("/");
    filename = parts[parts.length - 1];
  }

  const headers = {};
  if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

  const result = await sendDownload(url, filename, headers).catch(() => null);
  if (!result || !result.success) {
    // Fallback: restart download in Chrome normally
    // (omit filename when unknown — Chrome rejects null for optional string args)
    const opts = { url, saveAs: true };
    if (filename) opts.filename = filename;
    selfInitiated.add(url);
    chrome.downloads.download(opts);
  }
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
      .then((r) => sendResponse(r))
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getConfig") {
    sendResponse(config);
    return false;
  }
  if (message.action === "setConfig") {
    config = { ...config, ...message.config };
    chrome.storage.sync.set({ config });
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
