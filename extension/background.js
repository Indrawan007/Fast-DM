const NATIVE_HOST_NAME = "com.fastdm.native";

const DEFAULT_CONFIG = {
  enabled: true,
  interceptDownloads: true,
  interceptMinSize: 1048576,
  videoExtensions: [
    ".mp4", ".mkv", ".webm", ".avi", ".mov",
    ".flv", ".wmv", ".m4v", ".3gp", ".ts",
    ".m3u8", ".mpd"
  ],
  fileExtensions: [
    ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2",
    ".iso", ".dmg", ".exe", ".msi", ".deb", ".rpm",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx",
    ".mp3", ".flac", ".ogg", ".m4a", ".wav"
  ],
  excludePatterns: []
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

  chrome.storage.local.get("registered", (result) => {
    // Selalu kirim register saat startup untuk memastikan
    // manifest selalu up-to-date
    sendToNative({
      action: "register",
      extension_id: extId
    }).then((response) => {
      if (response && response.success) {
        chrome.storage.local.set({ registered: true });
        console.log("[FastDM] Extension registered:", extId);
      }
    }).catch((err) => {
      console.log("[FastDM] Register will retry on next connection:", err.message);
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
    registerExtensionId();
  }

  // Context menus
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
    } catch (err) { reject(err); }
  });
}

async function sendDownload(url, filename = null, headers = {}) {
  if (!filename) {
    try {
      const urlObj = new URL(url);
      const path = decodeURIComponent(urlObj.pathname);
      const parts = path.split("/").filter(Boolean);
      if (parts.length > 0) {
        const last = parts[parts.length - 1];
        if (last.includes(".")) filename = last;
      }
    } catch (e) { /* ignore */ }
  }

  const message = {
    action: "download",
    url: url,
    filename: filename,
    headers: headers,
    extension_id: chrome.runtime.id,
  };

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

chrome.downloads.onCreated.addListener((downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  if (shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime)) {
    chrome.downloads.cancel(downloadItem.id, () => {
      chrome.downloads.erase({ id: downloadItem.id });

      let filename = null;
      if (downloadItem.filename) {
        const parts = downloadItem.filename.replace(/\\/g, "/").split("/");
        filename = parts[parts.length - 1];
      }

      const headers = {};
      if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

      sendDownload(url, filename, headers);
    });
  }
});

function shouldInterceptUrl(url, fileSize, mimeType) {
  const urlLower = url.toLowerCase();

  for (const pattern of config.excludePatterns) {
    if (urlLower.includes(pattern)) return false;
  }

  try {
    const path = new URL(url).pathname.toLowerCase();
    const allExts = [...config.videoExtensions, ...config.fileExtensions];
    for (const ext of allExts) {
      if (path.endsWith(ext)) return true;
    }
  } catch (e) { /* ignore */ }

  if (mimeType) {
    const interceptMimes = [
      "video/", "audio/",
      "application/zip", "application/x-rar",
      "application/x-7z", "application/gzip",
      "application/pdf", "application/octet-stream",
      "application/x-iso9660-image",
      "application/x-bzip2", "application/x-tar",
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
    case "fastdm-download-link":  url = info.linkUrl; break;
    case "fastdm-download-video": url = info.srcUrl;  break;
    case "fastdm-download-image": url = info.srcUrl;  break;
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
  } catch (e) { /* ignore */ }

  sendDownload(url, filename, headers);
});


// ═══════════════════════════════════════════════
// Messages
// ═══════════════════════════════════════════════

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === "download") {
    sendDownload(message.url, message.filename, message.headers || {})
      .then(r => sendResponse(r))
      .catch(e => sendResponse({ success: false, error: e.message }));
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
      .then(r => sendResponse(r))
      .catch(e => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getStatus") {
    sendToNative({ action: "list" })
      .then(r => sendResponse(r))
      .catch(e => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getExtensionId") {
    sendResponse({ id: chrome.runtime.id });
    return false;
  }
});
