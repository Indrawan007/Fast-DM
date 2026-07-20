/**
 * Fast DM - Popup Logic
 */

document.addEventListener("DOMContentLoaded", () => {
  const urlInput = document.getElementById("url-input");
  const downloadBtn = document.getElementById("download-btn");
  const scanBtn = document.getElementById("scan-btn");
  const videoList = document.getElementById("video-list");
  const statusDot = document.getElementById("status-dot");
  const statusText = document.getElementById("status-text");
  const toggleIntercept = document.getElementById("toggle-intercept");
  const toggleEnabled = document.getElementById("toggle-enabled");

  // ===== Check Connection =====
  function checkConnection() {
    chrome.runtime.sendMessage({ action: "ping" }, (response) => {
      if (chrome.runtime.lastError || !response || !response.success) {
        statusDot.className = "status-dot disconnected";
        statusText.textContent = "Fast DM is not running";
      } else {
        statusDot.className = "status-dot connected";
        statusText.textContent = "Connected to Fast DM";
      }
    });
  }

  checkConnection();

  // ===== Download URL =====
  downloadBtn.addEventListener("click", () => {
    const url = urlInput.value.trim();
    if (!url) return;

    chrome.runtime.sendMessage(
      { action: "download", url: url },
      (response) => {
        if (response && response.success) {
          urlInput.value = "";
          urlInput.placeholder = "✓ Sent to Fast DM!";
          setTimeout(() => {
            urlInput.placeholder = "Paste download URL...";
          }, 2000);
        } else {
          urlInput.placeholder = "✕ Failed to send";
          setTimeout(() => {
            urlInput.placeholder = "Paste download URL...";
          }, 2000);
        }
      }
    );
  });

  urlInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") downloadBtn.click();
  });

  // Paste from clipboard on focus
  urlInput.addEventListener("focus", async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text && (text.startsWith("http://") || text.startsWith("https://"))) {
        if (!urlInput.value) {
          urlInput.value = text;
          urlInput.select();
        }
      }
    } catch (e) {
      // Clipboard permission denied - ignore
    }
  });

  // ===== Scan Videos =====
  scanBtn.addEventListener("click", () => {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      if (!tabs[0]) return;

      chrome.tabs.sendMessage(
        tabs[0].id,
        { action: "detectVideos" },
        (response) => {
          if (chrome.runtime.lastError) {
            videoList.innerHTML = '<div class="empty-text">Cannot scan this page</div>';
            return;
          }

          const videos = response?.videos || [];

          if (videos.length === 0) {
            videoList.innerHTML = '<div class="empty-text">No videos detected</div>';
            return;
          }

          videoList.innerHTML = "";
          videos.forEach((url) => {
            const item = document.createElement("div");
            item.className = "video-item";

            const name = document.createElement("span");
            name.className = "video-name";
            try {
              name.textContent = decodeURIComponent(
                new URL(url).pathname.split("/").pop()
              ) || url;
            } catch {
              name.textContent = url;
            }
            name.title = url;

            const btn = document.createElement("button");
            btn.className = "video-dl-btn";
            btn.textContent = "⬇ Download";
            btn.addEventListener("click", () => {
              chrome.runtime.sendMessage({
                action: "download",
                url: url,
                headers: { Referer: tabs[0].url },
              });
              btn.textContent = "✓ Sent";
              btn.disabled = true;
              setTimeout(() => {
                btn.textContent = "⬇ Download";
                btn.disabled = false;
              }, 2000);
            });

            item.appendChild(name);
            item.appendChild(btn);
            videoList.appendChild(item);
          });
        }
      );
    });
  });

  // ===== Settings =====
  chrome.runtime.sendMessage({ action: "getConfig" }, (cfg) => {
    if (cfg) {
      toggleIntercept.checked = cfg.interceptDownloads !== false;
      toggleEnabled.checked = cfg.enabled !== false;
    }
  });

  toggleIntercept.addEventListener("change", () => {
    chrome.runtime.sendMessage({
      action: "setConfig",
      config: { interceptDownloads: toggleIntercept.checked },
    });
  });

  toggleEnabled.addEventListener("change", () => {
    chrome.runtime.sendMessage({
      action: "setConfig",
      config: { enabled: toggleEnabled.checked },
    });
  });
});
