/**
 * Fast Download Manager - Background Service Worker
 *
 * Responsibilities:
 * 1. Intercept download events dari Chrome
 * 2. Detect video/large file links
 * 3. Forward ke Native Host (→ Python backend)
 * 4. Context menu integration
 */

const NATIVE_HOST_NAME = "com.fastdm.native";

// ========== Configuration ==========

const DEFAULT_CONFIG = {
  enabled: true,
  interceptDownloads: true,
  interceptMinSize: 1048576, // 1MB
  videoExtensions: [
    ".mp4", ".mkv", ".webm", ".avi", ".mov",
    ".flv", ".wmv", ".m4v", ".3gp", ".ts",
    ".m3u8", ".mpd"
  ],
  fileExtensions: [
    ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2",
    ".iso", ".dmg", ".exe", ".msi", ".deb", ".rpm",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx"
  ],
  excludePatterns: [
    "drive.google.com/uc", // Google Drive direct - handled separately
  ]
};

let config = { ...DEFAULT_CONFIG };

// Load saved config
chrome.storage.sync.get("config", (result) => {
  if (result.config) {
    config = { ...DEFAULT_CONFIG, ...result.config };
  }
});

// Listen for config changes
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "sync" && changes.config) {
    config = { ...DEFAULT_CONFIG, ...changes.config.newValue };
  }
});


// ========== Native Messaging ==========

/**
 * Kirim message ke native host.
 * Native host akan forward ke Python GUI via Unix socket.
 */
function sendToNative(message) {
  return new Promise((resolve, reject) => {
    try {
      chrome.runtime.sendNativeMessage(
        NATIVE_HOST_NAME,
        message,
        (response) => {
          if (chrome.runtime.lastError) {
            console.error(
              "[FastDM] Native messaging error:",
              chrome.runtime.lastError.message
            );
            reject(chrome.runtime.lastError);
            return;
          }
          resolve(response);
        }
      );
    } catch (err) {
      reject(err);
    }
  });
}

/**
 * Kirim URL ke Fast DM untuk didownload.
 */
async function sendDownload(url, filename = null, headers = {}) {
  const message = {
    action: "download",
    url: url,
    filename: filename,
    headers: headers,
  };

  try {
    const response = await sendToNative(message);
    console.log("[FastDM] Download sent:", url, response);

    // Show notification
    showNotification(
      "Download Started",
      filename || url.split("/").pop() || "Download"
    );

    return response;
  } catch (err) {
    console.error("[FastDM] Failed to send download:", err);
    showNotification(
      "Download Failed",
      "Could not connect to Fast DM. Is it running?"
    );
    return null;
  }
}

function showNotification(title, message) {
  // Service worker can't use chrome.notifications in all cases,
  // use badge instead
  chrome.action.setBadgeText({ text: "⬇" });
  chrome.action.setBadgeBackgroundColor({ color: "#89b4fa" });
  setTimeout(() => {
    chrome.action.setBadgeText({ text: "" });
  }, 3000);
}


// ========== Download Interception ==========

/**
 * Intercept Chrome downloads.
 * Saat Chrome mulai download, kita cancel dan redirect ke Fast DM.
 */
chrome.downloads.onCreated.addListener((downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  const url = downloadItem.url || downloadItem.finalUrl;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  // Cek apakah perlu di-intercept
  const shouldIntercept = shouldInterceptUrl(
    url,
    downloadItem.fileSize,
    downloadItem.mime
  );

  if (shouldIntercept) {
    // Cancel download Chrome
    chrome.downloads.cancel(downloadItem.id, () => {
      // Remove dari download list Chrome
      chrome.downloads.erase({ id: downloadItem.id });

      // Kirim ke Fast DM
      sendDownload(
        url,
        downloadItem.filename ? downloadItem.filename.split("/").pop() : null,
        buildHeaders(downloadItem)
      );
    });
  }
});

/**
 * Tentukan apakah URL harus di-intercept.
 */
function shouldInterceptUrl(url, fileSize, mimeType) {
  const urlLower = url.toLowerCase();

  // Exclude patterns
  for (const pattern of config.excludePatterns) {
    if (urlLower.includes(pattern)) return false;
  }

  // Cek extension
  const path = new URL(url).pathname.toLowerCase();
  const allExtensions = [...config.videoExtensions, ...config.fileExtensions];

  for (const ext of allExtensions) {
    if (path.endsWith(ext)) return true;
  }

  // Cek MIME type
  if (mimeType) {
    const interceptMimes = [
      "video/", "audio/",
      "application/zip", "application/x-rar",
      "application/x-7z", "application/gzip",
      "application/pdf", "application/octet-stream",
      "application/x-iso9660-image",
    ];
    for (const mime of interceptMimes) {
      if (mimeType.startsWith(mime)) return true;
    }
  }

  // Cek file size (jika diketahui)
  if (fileSize && fileSize > config.interceptMinSize) {
    return true;
  }

  return false;
}

/**
 * Build headers dari download item.
 */
function buildHeaders(downloadItem) {
  const headers = {};

  if (downloadItem.referrer) {
    headers["Referer"] = downloadItem.referrer;
  }

  // Cookies akan diambil dari content script
  return headers;
}


// ========== Context Menu ==========

// Create context menu on install
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

  if (url) {
    // Get cookies dan headers dari tab
    const headers = {};
    if (info.pageUrl) {
      headers["Referer"] = info.pageUrl;
    }

    // Extract filename dari URL
    try {
      const path = new URL(url).pathname;
      filename = decodeURIComponent(path.split("/").pop()) || null;
    } catch (e) {
      // ignore
    }

    sendDownload(url, filename, headers);
  }
});


// ========== Message from Popup/Content Script ==========

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === "download") {
    sendDownload(message.url, message.filename, message.headers || {})
      .then((response) => sendResponse(response))
      .catch((err) => sendResponse({ success: false, error: err.message }));
    return true; // Async response
  }

  if (message.action === "getConfig") {
    sendResponse(config);
    return false;
  }

  if (message.action === "setConfig") {
    config = { ...config, ...message.config };
    chrome.storage.sync.set({ config: config });
    sendResponse({ success: true });
    return false;
  }

  if (message.action === "ping") {
    sendToNative({ action: "ping" })
      .then((response) => sendResponse(response))
      .catch((err) => sendResponse({ success: false, error: err.message }));
    return true;
  }

  if (message.action === "getStatus") {
    sendToNative({ action: "list" })
      .then((response) => sendResponse(response))
      .catch((err) => sendResponse({ success: false, error: err.message }));
    return true;
  }
});
