/**
 * Fast DM — Media Sniffer (gaya IDM Grabber)
 *
 * Berjalan di document_start (sebelum halaman load) supaya bisa menangkap
 * permintaan media sedini mungkin:
 * - hook fetch() dan XMLHttpRequest → kumpulkan URL .m3u8/.mpd/.mp4/.webm/dll
 * - scan element <video>/<audio>/<source>/<a href>
 *
 * Hasil disimpan di window.__fastdmMediaCandidates (array, paling baru terakhir).
 * content.js membacanya saat tombol "Unduh" diklik pada video blob:/streaming.
 */
(() => {
  "use strict";

  const MEDIA_RE =
    /\.(m3u8|mpd|mp4|webm|mkv|m4v|mov|flv|wmv|mp3|m4a|aac|ogg|opus|flac)([?#].*)?$/i;

  const candidates = new Set();
  const MAX = 50;

  function add(url) {
    try {
      if (!url || typeof url !== "string") return;
      if (!/^https?:/i.test(url)) return; // abaikan blob:, data:, file:
      const clean = url.split("#")[0];
      if (!MEDIA_RE.test(clean)) return;
      candidates.add(clean);
      if (candidates.size > MAX) {
        const first = candidates.values().next().value;
        candidates.delete(first);
      }
      window.__fastdmMediaCandidates = Array.from(candidates);
    } catch (e) {
      /* ignore */
    }
  }

  // ── 1. Hook fetch ──
  const origFetch = window.fetch;
  window.fetch = function (input, init) {
    try {
      if (typeof input === "string") add(input);
      else if (input && input.url) add(input.url);
    } catch (e) {
      /* ignore */
    }
    return origFetch.apply(this, arguments);
  };

  // ── 2. Hook XMLHttpRequest ──
  const origOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    try {
      add(String(url));
    } catch (e) {
      /* ignore */
    }
    return origOpen.apply(this, arguments);
  };

  // ── 3. Scan DOM (dipanggil saat DOM siap + setiap mutasi) ──
  function scan() {
    const els = document.querySelectorAll("video, audio, source");
    for (const el of els) {
      add(el.src);
      add(el.currentSrc);
    }
    document.querySelectorAll("a[href]").forEach((a) => add(a.href));
  }

  function start() {
    scan();
    const mo = new MutationObserver(() => scan());
    mo.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["src"],
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
