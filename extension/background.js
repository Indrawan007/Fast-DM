/**
 * Fast Download Manager - Background Service Worker
 * v2: Mengirim filename dari Content-Disposition & URL ke native host
 */

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


// ========== Native Messaging ==========

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

/**
 * Kirim download ke Fast DM dengan nama file yang benar.
 */
async function sendDownload(url, filename = null, headers = {}) {
  // Jika filename belum ada, extract dari URL
  if (!filename) {
    try {
      const urlObj = new URL(url);
      const path = decodeURIComponent(urlObj.pathname);
      const parts = path.split("/").filter(Boolean);
      if (parts.length > 0) {
        const last = parts[parts.length - 1];
        // Hanya pakai jika terlihat seperti nama file (ada ekstensi)
        if (last.includes(".")) {
          filename = last;
        }
      }
    } catch (e) { /* ignore */ }
  }

  const message = {
    action: "download",
    url: url,
    filename: filename,
    headers: headers,
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


// ========== Download Interception ==========

chrome.downloads.onCreated.addListener((downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  if (shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime)) {
    chrome.downloads.cancel(downloadItem.id, () => {
      chrome.downloads.erase({ id: downloadItem.id });

      // ── Extract filename dari browser download item ──
      let filename = null;

      // Chrome menyediakan filename di downloadItem
      if (downloadItem.filename) {
        // downloadItem.filename berisi full path, ambil basename
        const parts = downloadItem.filename.replace(/\\/g, "/").split("/");
        filename = parts[parts.length - 1];
      }

      // Headers dari browser
      const headers = {};
      if (downloadItem.referrer) {
        headers["Referer"] = downloadItem.referrer;
      }

      sendDownload(url, filename, headers);
    });
  }
});

function shouldInterceptUrl(url, fileSize, mimeType) {
  const urlLower = url.toLowerCase();

  for (const pattern of config.excludePatterns) {
    if (urlLower.includes(pattern)) return false;
  }

  // Cek ekstensi di URL path (bukan query string)
  try {
    const path = new URL(url).pathname.toLowerCase();
    const allExts = [...config.videoExtensions, ...config.fileExtensions];
    for (const ext of allExts) {
      if (path.endsWith(ext)) return true;
    }
  } catch (e) { /* ignore */ }

  // Cek MIME type
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

  // Cek file size
  if (fileSize && fileSize > config.interceptMinSize) return true;

  return false;
}


// ========== Context Menu ==========

chrome.runtime.onInstalled.addListener(() => {
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

  // Extract filename dari URL (bersih, tanpa query string)
  try {
    const path = new URL(url).pathname;
    const decoded = decodeURIComponent(path);
    const parts = decoded.split("/").filter(Boolean);
    if (parts.length > 0) {
      const last = parts[parts.length - 1];
      if (last && last.includes(".")) filename = last;
    }
  } catch (e) { /* ignore */ }

  // Untuk image, jika tidak ada filename, coba dari alt text
  if (!filename && info.menuItemId === "fastdm-download-image") {
    // Fallback
    filename = null;
  }

  sendDownload(url, filename, headers);
});


// ========== Messages ==========

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
});
