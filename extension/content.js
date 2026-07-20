/**
 * Content Script - Detect video elements and downloadable links.
 *
 * Runs pada setiap page untuk:
 * 1. Detect <video> dan <source> elements
 * 2. Detect links ke file downloadable
 * 3. Inject "Download with Fast DM" button pada video players
 */

(() => {
  "use strict";

  // Debounce untuk menghindari spam
  let detectionTimeout = null;

  /**
   * Scan page untuk video sources.
   */
  function detectVideos() {
    const videos = new Set();

    // <video> elements
    document.querySelectorAll("video").forEach((video) => {
      if (video.src && !video.src.startsWith("blob:")) {
        videos.add(video.src);
      }
      // <source> children
      video.querySelectorAll("source").forEach((source) => {
        if (source.src && !source.src.startsWith("blob:")) {
          videos.add(source.src);
        }
      });
    });

    // <a> links to video files
    const videoExts = /\.(mp4|mkv|webm|avi|mov|flv|wmv|m4v|3gp|ts)(\?|$)/i;
    document.querySelectorAll("a[href]").forEach((a) => {
      if (videoExts.test(a.href)) {
        videos.add(a.href);
      }
    });

    return Array.from(videos);
  }

  /**
   * Inject download button pada video elements.
   */
  function injectVideoButtons() {
    document.querySelectorAll("video").forEach((video) => {
      // Skip jika sudah ada button
      if (video.dataset.fastdmInjected) return;
      video.dataset.fastdmInjected = "true";

      const src = video.src || video.querySelector("source")?.src;
      if (!src || src.startsWith("blob:")) return;

      // Create overlay button
      const wrapper = video.parentElement;
      if (!wrapper) return;

      const btn = document.createElement("button");
      btn.textContent = "⚡ Fast DM";
      btn.title = "Download with Fast Download Manager";
      Object.assign(btn.style, {
        position: "absolute",
        top: "10px",
        right: "10px",
        zIndex: "9999",
        padding: "6px 12px",
        backgroundColor: "rgba(137, 180, 250, 0.9)",
        color: "#1e1e2e",
        border: "none",
        borderRadius: "6px",
        cursor: "pointer",
        fontSize: "12px",
        fontWeight: "bold",
        opacity: "0",
        transition: "opacity 0.2s",
        pointerEvents: "auto",
      });

      // Show on hover
      const showBtn = () => { btn.style.opacity = "1"; };
      const hideBtn = () => { btn.style.opacity = "0"; };

      wrapper.addEventListener("mouseenter", showBtn);
      wrapper.addEventListener("mouseleave", hideBtn);

      btn.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();

        chrome.runtime.sendMessage({
          action: "download",
          url: src,
          headers: { Referer: window.location.href },
        });

        btn.textContent = "✓ Sent!";
        setTimeout(() => { btn.textContent = "⚡ Fast DM"; }, 2000);
      });

      // Make wrapper relative if not already positioned
      const wrapperPos = getComputedStyle(wrapper).position;
      if (wrapperPos === "static") {
        wrapper.style.position = "relative";
      }

      wrapper.appendChild(btn);
    });
  }

  /**
   * Observe DOM changes untuk video yang dimuat secara dinamis.
   */
  function startObserving() {
    const observer = new MutationObserver((mutations) => {
      let hasRelevant = false;
      for (const mutation of mutations) {
        for (const node of mutation.addedNodes) {
          if (node.nodeType === 1) {
            if (
              node.tagName === "VIDEO" ||
              node.tagName === "SOURCE" ||
              node.querySelector?.("video, source")
            ) {
              hasRelevant = true;
              break;
            }
          }
        }
        if (hasRelevant) break;
      }

      if (hasRelevant) {
        clearTimeout(detectionTimeout);
        detectionTimeout = setTimeout(() => {
          injectVideoButtons();
        }, 500);
      }
    });

    observer.observe(document.body, {
      childList: true,
      subtree: true,
    });
  }

  // Initial scan
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => {
      injectVideoButtons();
      startObserving();
    });
  } else {
    injectVideoButtons();
    startObserving();
  }

  // Listen for messages from popup
  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message.action === "detectVideos") {
      sendResponse({ videos: detectVideos() });
    }
    if (message.action === "getCookies") {
      sendResponse({ cookies: document.cookie });
    }
  });
})();
