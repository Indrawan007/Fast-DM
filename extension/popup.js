document.addEventListener("DOMContentLoaded", () => {
  const urlInput        = document.getElementById("url-input");
  const downloadBtn     = document.getElementById("download-btn");
  const scanBtn         = document.getElementById("scan-btn");
  const videoList       = document.getElementById("video-list");
  const statusDot       = document.getElementById("status-dot");
  const statusText      = document.getElementById("status-text");
  const feedback        = document.getElementById("action-feedback");
  const toggleIntercept = document.getElementById("toggle-intercept");
  const toggleEnabled   = document.getElementById("toggle-enabled");

  // ── C5: feedback persisten (bukan menyalahgunakan placeholder) ──
  let feedbackTimer = null;
  function setFeedback(msg, ok) {
    feedback.textContent = msg;
    feedback.className = "action-feedback " + (ok ? "ok" : "err");
    feedback.hidden = false;
    clearTimeout(feedbackTimer);
    feedbackTimer = setTimeout(() => { feedback.hidden = true; }, 3500);
  }

  // ── Check Connection + Auto Register ──
  function checkConnection() {
    chrome.runtime.sendMessage({ action: "ping" }, (response) => {
      if (chrome.runtime.lastError || !response || !response.success) {
        statusDot.className = "status-dot disconnected";
        statusText.textContent = "Fast DM tidak berjalan. Jalankan: fast-dm";
      } else {
        statusDot.className = "status-dot connected";
        statusText.textContent = "Terhubung ke Fast DM ✓";
      }
    });
  }

  checkConnection();

  // ── Download URL ──
  downloadBtn.addEventListener("click", () => {
    const url = urlInput.value.trim();
    if (!url) return;

    chrome.runtime.sendMessage(
      { action: "download", url: url },
      (response) => {
        if (response && response.success) {
          urlInput.value = "";
          setFeedback("✓ Terkirim ke Fast DM!", true);
        } else {
          setFeedback("✕ Gagal — pastikan Fast DM berjalan", false);
        }
      }
    );
  });

  urlInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") downloadBtn.click();
  });

  // Auto-paste
  urlInput.addEventListener("focus", async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text && (text.startsWith("http://") || text.startsWith("https://"))) {
        if (!urlInput.value) {
          urlInput.value = text;
          urlInput.select();
        }
      }
    } catch (e) { /* ignore */ }
  });

  // ── Scan Videos ──
  function scanVideos() {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      if (!tabs[0]) return;

      chrome.tabs.sendMessage(
        tabs[0].id,
        { action: "detectVideos" },
        (response) => {
          if (chrome.runtime.lastError) {
            videoList.innerHTML =
              '<div class="empty-text">Tidak bisa memindai halaman ini</div>';
            return;
          }

          const videos = response?.videos || [];

          if (videos.length === 0) {
            videoList.innerHTML =
              '<div class="empty-text">Tidak ada video terdeteksi</div>';
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
            btn.textContent = "⬇ Unduh";
            btn.addEventListener("click", () => {
              chrome.runtime.sendMessage({
                action: "download",
                url: url,
                headers: { Referer: tabs[0].url },
              });
              btn.textContent = "✓ Terkirim";
              btn.disabled = true;
              setTimeout(() => {
                btn.textContent = "⬇ Unduh";
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
  }

  scanBtn.addEventListener("click", scanVideos);

  // ── C4: scan otomatis saat popup dibuka (tombol tetap ada untuk scan ulang) ──
  setTimeout(scanVideos, 150);

  // ── Settings ──
  chrome.runtime.sendMessage({ action: "getConfig" }, (cfg) => {
    if (cfg) {
      toggleIntercept.checked = cfg.interceptDownloads !== false;
      toggleEnabled.checked   = cfg.enabled !== false;
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
