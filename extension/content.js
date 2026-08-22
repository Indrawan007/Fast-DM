/**
 * Fast DM Pro — YouTube Overlay (IDM-like)
 *
 * Fitur:
 * - Tombol download di pojok video player
 * - Dropdown pilih kualitas langsung dari overlay
 * - Support Watch, Shorts, Embed
 * - Auto-detect video berubah (YouTube SPA)
 * - Download audio langsung
 * - Playlist button
 * - Smooth animation
 */

(() => {
  "use strict";

  // ═══════════════════════════════════════════════
  // Config
  // ═══════════════════════════════════════════════

  const YT_HOSTS = [
    "youtube.com", "www.youtube.com",
    "m.youtube.com", "music.youtube.com"
  ];

  const QUALITIES = [
    { id: "best_mp4",   label: "Best Quality",   icon: "🎬", desc: "MP4" },
    { id: "2160p",      label: "4K Ultra HD",     icon: "📺", desc: "2160p" },
    { id: "1440p",      label: "2K QHD",          icon: "📺", desc: "1440p" },
    { id: "1080p",      label: "Full HD",         icon: "📺", desc: "1080p" },
    { id: "720p",       label: "HD",              icon: "📺", desc: "720p" },
    { id: "480p",       label: "SD",              icon: "📺", desc: "480p" },
    { id: "360p",       label: "Low",             icon: "📺", desc: "360p" },
    { id: "audio_best", label: "Audio M4A",       icon: "🎵", desc: "Best" },
    { id: "audio_mp3",  label: "Audio MP3",       icon: "🎵", desc: "320kbps" },
  ];

  // ═══════════════════════════════════════════════
  // State
  // ═══════════════════════════════════════════════

  let currentVideoId = null;
  let overlayContainer = null;
  let dropdownVisible = false;
  let hideTimeout = null;

  // ═══════════════════════════════════════════════
  // Utility
  // ═══════════════════════════════════════════════

  function isYouTube() {
    return YT_HOSTS.includes(window.location.hostname);
  }

  function extractVideoId() {
    const url = new URL(window.location.href);
    if (url.searchParams.get("v")) return url.searchParams.get("v");
    const shorts = url.pathname.match(/\/shorts\/([a-zA-Z0-9_-]+)/);
    if (shorts) return shorts[1];
    const embed = url.pathname.match(/\/embed\/([a-zA-Z0-9_-]+)/);
    if (embed) return embed[1];
    return null;
  }

  function getVideoUrl(videoId) {
    return "https://www.youtube.com/watch?v=" + videoId;
  }

  function sendDownload(url, quality) {
    chrome.runtime.sendMessage({
      action: "download",
      url: url,
      headers: {},
      quality: quality || "best_mp4"
    }, (response) => {
      if (chrome.runtime.lastError || !response || !response.success) {
        showToast("Failed to send to Fast DM");
      } else {
        showToast("Sent to Fast DM — " + (quality || "best_mp4"));
      }
    });
  }

  // ═══════════════════════════════════════════════
  // Styles (injected once)
  // ═══════════════════════════════════════════════

  function injectStyles() {
    if (document.getElementById("fastdm-styles")) return;

    const style = document.createElement("style");
    style.id = "fastdm-styles";
    style.textContent = `
      /* ── Container ── */
      .fastdm-overlay {
        position: absolute;
        top: 12px;
        right: 12px;
        z-index: 999999;
        opacity: 0;
        transition: opacity 0.25s ease;
        pointer-events: none;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      }
      .fastdm-overlay.visible {
        opacity: 1;
        pointer-events: auto;
      }

      /* ── Main Button ── */
      .fastdm-btn {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 8px 14px;
        background: linear-gradient(135deg, #89b4fa 0%, #74c7ec 100%);
        color: #11111b;
        border: none;
        border-radius: 10px;
        font-weight: 800;
        font-size: 13px;
        cursor: pointer;
        box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
        transition: all 0.15s ease;
        white-space: nowrap;
        user-select: none;
        letter-spacing: 0.3px;
      }
      .fastdm-btn:hover {
        background: linear-gradient(135deg, #b4d0fb 0%, #89dceb 100%);
        transform: scale(1.05);
        box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
      }
      .fastdm-btn:active {
        transform: scale(0.98);
      }
      .fastdm-btn .fastdm-icon {
        font-size: 15px;
        line-height: 1;
      }
      .fastdm-btn .fastdm-arrow {
        font-size: 10px;
        transition: transform 0.2s ease;
      }
      .fastdm-btn .fastdm-arrow.open {
        transform: rotate(180deg);
      }

      /* ── Dropdown ── */
      .fastdm-dropdown {
        position: absolute;
        top: calc(100% + 6px);
        right: 0;
        min-width: 220px;
        background: rgba(30, 30, 46, 0.97);
        backdrop-filter: blur(16px);
        border: 1px solid rgba(137, 180, 250, 0.2);
        border-radius: 12px;
        padding: 6px;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.7);
        opacity: 0;
        transform: translateY(-8px) scale(0.95);
        transition: all 0.2s ease;
        pointer-events: none;
        overflow: hidden;
      }
      .fastdm-dropdown.open {
        opacity: 1;
        transform: translateY(0) scale(1);
        pointer-events: auto;
      }

      /* ── Dropdown Header ── */
      .fastdm-dd-header {
        padding: 8px 12px 6px;
        font-size: 10px;
        font-weight: 700;
        color: #585b70;
        text-transform: uppercase;
        letter-spacing: 1.5px;
      }

      /* ── Dropdown Items ── */
      .fastdm-dd-item {
        display: flex;
        align-items: center;
        gap: 10px;
        width: 100%;
        padding: 9px 12px;
        background: transparent;
        border: none;
        border-radius: 8px;
        color: #cdd6f4;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.12s ease;
        text-align: left;
      }
      .fastdm-dd-item:hover {
        background: rgba(137, 180, 250, 0.12);
        color: #89b4fa;
      }
      .fastdm-dd-item:active {
        background: rgba(137, 180, 250, 0.2);
        transform: scale(0.98);
      }
      .fastdm-dd-item .dd-icon {
        font-size: 16px;
        width: 20px;
        text-align: center;
        flex-shrink: 0;
      }
      .fastdm-dd-item .dd-label {
        flex: 1;
      }
      .fastdm-dd-item .dd-desc {
        font-size: 11px;
        color: #585b70;
        font-weight: 500;
      }

      /* ── Separator ── */
      .fastdm-dd-sep {
        height: 1px;
        background: rgba(69, 71, 90, 0.5);
        margin: 4px 8px;
      }

      /* ── Toast ── */
      .fastdm-toast {
        position: fixed;
        bottom: 24px;
        right: 24px;
        z-index: 9999999;
        padding: 12px 20px;
        background: rgba(30, 30, 46, 0.95);
        backdrop-filter: blur(12px);
        border: 1px solid rgba(166, 227, 161, 0.3);
        border-radius: 10px;
        color: #a6e3a1;
        font-family: -apple-system, BlinkMacSystemFont, sans-serif;
        font-size: 13px;
        font-weight: 700;
        box-shadow: 0 8px 24px rgba(0,0,0,0.5);
        opacity: 0;
        transform: translateY(16px);
        transition: all 0.3s ease;
        pointer-events: none;
      }
      .fastdm-toast.show {
        opacity: 1;
        transform: translateY(0);
      }

      /* ── Shorts overlay position ── */
      .fastdm-overlay.shorts {
        top: auto;
        bottom: 120px;
        right: 16px;
      }
    `;
    document.head.appendChild(style);
  }

  // ═══════════════════════════════════════════════
  // Toast Notification
  // ═══════════════════════════════════════════════

  function showToast(message) {
    let toast = document.querySelector(".fastdm-toast");
    if (!toast) {
      toast = document.createElement("div");
      toast.className = "fastdm-toast";
      document.body.appendChild(toast);
    }

    toast.textContent = message;
    toast.classList.remove("show");

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        toast.classList.add("show");
      });
    });

    setTimeout(() => {
      toast.classList.remove("show");
    }, 2500);
  }

  // ═══════════════════════════════════════════════
  // Overlay Builder
  // ═══════════════════════════════════════════════

  function createOverlay(isShorts) {
    if (overlayContainer) overlayContainer.remove();

    const container = document.createElement("div");
    container.className = "fastdm-overlay" + (isShorts ? " shorts" : "");

    // ── Main Button ──
    const btn = document.createElement("button");
    btn.className = "fastdm-btn";
    btn.innerHTML = `
      <span class="fastdm-icon">⚡</span>
      <span>Download</span>
      <span class="fastdm-arrow">▾</span>
    `;

    // ── Dropdown ──
    const dropdown = document.createElement("div");
    dropdown.className = "fastdm-dropdown";

    // Header: Video
    const headerVideo = document.createElement("div");
    headerVideo.className = "fastdm-dd-header";
    headerVideo.textContent = "VIDEO";
    dropdown.appendChild(headerVideo);

    // Video qualities
    QUALITIES.filter(q => !q.id.startsWith("audio")).forEach(q => {
      dropdown.appendChild(createDropdownItem(q));
    });

    // Separator
    const sep = document.createElement("div");
    sep.className = "fastdm-dd-sep";
    dropdown.appendChild(sep);

    // Header: Audio
    const headerAudio = document.createElement("div");
    headerAudio.className = "fastdm-dd-header";
    headerAudio.textContent = "AUDIO ONLY";
    dropdown.appendChild(headerAudio);

    // Audio options
    QUALITIES.filter(q => q.id.startsWith("audio")).forEach(q => {
      dropdown.appendChild(createDropdownItem(q));
    });

    container.appendChild(btn);
    container.appendChild(dropdown);

    // ── Button click: toggle dropdown ──
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      e.preventDefault();
      dropdownVisible = !dropdownVisible;
      dropdown.classList.toggle("open", dropdownVisible);
      btn.querySelector(".fastdm-arrow").classList.toggle("open", dropdownVisible);
    });

    // ── Close dropdown on outside click ──
    document.addEventListener("click", (e) => {
      if (!container.contains(e.target)) {
        dropdownVisible = false;
        dropdown.classList.remove("open");
        btn.querySelector(".fastdm-arrow").classList.remove("open");
      }
    });

    overlayContainer = container;
    return container;
  }

  function createDropdownItem(quality) {
    const item = document.createElement("button");
    item.className = "fastdm-dd-item";
    item.innerHTML = `
      <span class="dd-icon">${quality.icon}</span>
      <span class="dd-label">${quality.label}</span>
      <span class="dd-desc">${quality.desc}</span>
    `;

    let sending = false;  // Debounce

    item.addEventListener("click", (e) => {
      e.stopPropagation();
      e.preventDefault();

      if (sending) return;  // Prevent double click
      sending = true;

      const videoId = extractVideoId();
      if (!videoId) {
        showToast("Cannot detect video");
        sending = false;
        return;
      }

      const url = getVideoUrl(videoId);

      chrome.runtime.sendMessage({
        action: "download",
        url: url,
        quality: quality.id
      }, (response) => {
        if (chrome.runtime.lastError || !response || !response.success) {
          showToast("Failed to send to Fast DM");
        } else {
          showToast("Sent to Fast DM — " + quality.label);
        }
      });

      setTimeout(() => { sending = false; }, 2000);
    });

    return item;
  }

  // ═══════════════════════════════════════════════
  // Attach Overlay to Player
  // ═══════════════════════════════════════════════

  function attachOverlay() {
    const videoId = extractVideoId();
    if (!videoId) return;

    const isShorts = window.location.pathname.includes("/shorts/");

    // Cari player container
    let player = null;

    if (isShorts) {
      // Shorts: cari player aktif
      player = document.querySelector(
        "ytd-reel-video-renderer[is-active] .html5-video-player"
      ) || document.querySelector(
        "#shorts-player .html5-video-player"
      ) || document.querySelector(
        ".html5-video-player"
      );
    } else {
      // Watch page
      player = document.querySelector(
        "#movie_player"
      ) || document.querySelector(
        ".html5-video-player"
      );
    }

    if (!player) return;

    // Sudah ada overlay di player ini?
    if (player.querySelector(".fastdm-overlay")) return;

    const overlay = createOverlay(isShorts);

    // Pastikan player relative
    const pos = getComputedStyle(player).position;
    if (pos === "static") player.style.position = "relative";

    player.appendChild(overlay);

    // Show/hide on hover
    player.addEventListener("mouseenter", () => {
      clearTimeout(hideTimeout);
      overlay.classList.add("visible");
    });

    player.addEventListener("mouseleave", () => {
      hideTimeout = setTimeout(() => {
        if (!dropdownVisible) {
          overlay.classList.remove("visible");
        }
      }, 600);
    });

    // Keep visible while dropdown open
    overlay.addEventListener("mouseenter", () => {
      clearTimeout(hideTimeout);
      overlay.classList.add("visible");
    });

    overlay.addEventListener("mouseleave", () => {
      if (!dropdownVisible) {
        hideTimeout = setTimeout(() => {
          overlay.classList.remove("visible");
        }, 400);
      }
    });
  }

  // ═══════════════════════════════════════════════
  // Detect video & non-YouTube pages
  // ═══════════════════════════════════════════════

  function detectNonYTVideos() {
    if (isYouTube()) return;

    document.querySelectorAll("video").forEach((video) => {
      if (video.dataset.fastdmDone) return;
      video.dataset.fastdmDone = "true";

      const src = video.src || video.querySelector("source")?.src;
      if (!src || src.startsWith("blob:") || src.startsWith("data:")) return;

      const wrapper = video.parentElement;
      if (!wrapper) return;

      const btn = document.createElement("button");
      btn.className = "fastdm-btn";
      btn.innerHTML = '<span class="fastdm-icon">⚡</span><span>Download</span>';

      Object.assign(btn.style, {
        position: "absolute",
        top: "10px",
        right: "10px",
        zIndex: "999999",
        opacity: "0",
        transition: "opacity 0.2s ease",
      });

      wrapper.addEventListener("mouseenter", () => btn.style.opacity = "1");
      wrapper.addEventListener("mouseleave", () => btn.style.opacity = "0");

      btn.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();
        chrome.runtime.sendMessage({
          action: "download",
          url: src,
          headers: { Referer: window.location.href }
        }, (response) => {
          if (chrome.runtime.lastError || !response || !response.success) {
            showToast("Failed to send to Fast DM");
          } else {
            showToast("Sent to Fast DM");
          }
        });
      });

      const wPos = getComputedStyle(wrapper).position;
      if (wPos === "static") wrapper.style.position = "relative";

      wrapper.appendChild(btn);
    });

    // Detect YouTube embeds
    document.querySelectorAll("iframe").forEach((iframe) => {
      const src = iframe.src || "";
      if (!src.includes("youtube.com/embed/")) return;
      if (iframe.dataset.fastdmDone) return;
      iframe.dataset.fastdmDone = "true";

      const m = src.match(/embed\/([a-zA-Z0-9_-]+)/);
      if (!m) return;

      const videoUrl = "https://www.youtube.com/watch?v=" + m[1];
      const wrapper = iframe.parentElement;
      if (!wrapper) return;

      const btn = document.createElement("button");
      btn.className = "fastdm-btn";
      btn.innerHTML = '<span class="fastdm-icon">⚡</span><span>Download</span>';

      Object.assign(btn.style, {
        position: "absolute",
        top: "10px",
        right: "10px",
        zIndex: "999999",
        opacity: "0",
        transition: "opacity 0.2s ease",
      });

      wrapper.addEventListener("mouseenter", () => btn.style.opacity = "1");
      wrapper.addEventListener("mouseleave", () => btn.style.opacity = "0");

      btn.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();
        chrome.runtime.sendMessage({ action: "download", url: videoUrl }, (response) => {
          if (chrome.runtime.lastError || !response || !response.success) {
            showToast("Failed to send to Fast DM");
          } else {
            showToast("Sent to Fast DM");
          }
        });
      });

      const wPos = getComputedStyle(wrapper).position;
      if (wPos === "static") wrapper.style.position = "relative";
      wrapper.appendChild(btn);
    });
  }

  // ═══════════════════════════════════════════════
  // Observer — YouTube SPA navigation
  // ═══════════════════════════════════════════════

  function startObserver() {
    let lastUrl = location.href;
    let scanTimer = null;

    const observer = new MutationObserver(() => {
      // Collapse mutation bursts → max 1 scan per 400ms (CPU savings on heavy SPA pages)
      if (scanTimer) return;
      scanTimer = setTimeout(() => {
        scanTimer = null;

        if (location.href !== lastUrl) {
          lastUrl = location.href;
          const newId = extractVideoId();
          if (newId !== currentVideoId) {
            currentVideoId = newId;
            // Cleanup overlay dan dropdown
            dropdownVisible = false;
            if (overlayContainer) {
              overlayContainer.remove();
              overlayContainer = null;
            }
          }
        }

        if (isYouTube()) {
          attachOverlay();
        } else {
          detectNonYTVideos();
        }
      }, 400);
    });

    observer.observe(document.body, {
      childList: true,
      subtree: true,
    });
  }

  // ═══════════════════════════════════════════════
  // Messages from popup
  // ═══════════════════════════════════════════════

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message.action === "detectVideos") {
      const videos = new Set();

      // HTML5 videos
      document.querySelectorAll("video").forEach(v => {
        if (v.src && !v.src.startsWith("blob:")) videos.add(v.src);
        v.querySelectorAll("source").forEach(s => {
          if (s.src && !s.src.startsWith("blob:")) videos.add(s.src);
        });
      });

      // YouTube embeds
      document.querySelectorAll("iframe").forEach(iframe => {
        const src = iframe.src || "";
        const m = src.match(/embed\/([a-zA-Z0-9_-]+)/);
        if (m) videos.add("https://www.youtube.com/watch?v=" + m[1]);
      });

      // Current YouTube video
      const vid = extractVideoId();
      if (vid) videos.add(getVideoUrl(vid));

      // Video links
      const videoExts = /\.(mp4|mkv|webm|avi|mov|flv|wmv|m4v|3gp|ts)(\?|$)/i;
      document.querySelectorAll("a[href]").forEach(a => {
        if (videoExts.test(a.href)) videos.add(a.href);
      });

      sendResponse({ videos: Array.from(videos) });
    }

    if (message.action === "getCookies") {
      sendResponse({ cookies: document.cookie });
    }
  });

  // ═══════════════════════════════════════════════
  // Init
  // ═══════════════════════════════════════════════

  injectStyles();

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => {
      startObserver();
      if (isYouTube()) attachOverlay();
      else detectNonYTVideos();
    });
  } else {
    startObserver();
    setTimeout(() => {
      if (isYouTube()) attachOverlay();
      else detectNonYTVideos();
    }, 1000);
  }
})();
