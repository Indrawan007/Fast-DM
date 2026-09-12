/**
 * Fast DM — Media Sniffer (gaya IDM Grabber)
 *
 * Berjalan di document_start (sebelum halaman load) supaya bisa menangkap
 * permintaan media sedini mungkin:
 * - hook fetch() dan XMLHttpRequest → kumpulkan URL .m3u8/.mpd/.mp4/.webm/dll
 * - scan element <video>/<audio>/<source>/<a href>
 *
 * Hasil disimpan di document.documentElement.dataset.fastdmMedia (JSON array,
 * paling baru terakhir). Script ini berjalan di MAIN world agar hook
 * fetch/XHR menangkap request MILIK HALAMAN; content.js (ISOLATED world)
 * membacanya lewat DOM — satu-satunya jembatan antar world.
 *
 * v2.9.4 (B4): script ini di-inject ke SETIAP halaman, jadi biayanya harus
 * mendekati nol di halaman yang tidak punya media. Tiga perubahan:
 *
 *  a. `persist()` di-batch per microtask dan hanya jalan bila kandidat
 *     benar-benar bertambah — dulu seluruh Set di-serialize ulang pada setiap
 *     `add()`, puluhan kali per burst mutasi.
 *  b. MutationObserver hanya memindai node yang BARU ditambahkan (atau yang
 *     atribut `src`-nya berubah) — dulu `querySelectorAll` atas SELURUH
 *     dokumen tiap burst 300 ms, sehingga halaman berisi ribuan `a[href]`
 *     membayar biaya penuh berulang kali.
 *  c. "Dormant": bila sampai IDLE_MS setelah `load` tidak ada satu pun sinyal
 *     media, pemindaian ditidurkan dan callback observer tinggal melakukan
 *     pemeriksaan murah. Tidur bukan permanen — bangun lagi otomatis bila URL
 *     berubah (navigasi SPA) atau ada node media/link media baru.
 *
 * Catatan (c): saat dormant, `<a href="…mp4">` yang muncul DI DALAM subtree
 * baru (bukan sebagai node yang ditambahkan langsung) tidak membangunkan
 * sniffer. Jalur pemulihannya tetap ada: tombol "Pindai" di popup memakai
 * `detectVideos` di content.js, yang menyapu `a[href]` seluruh dokumen secara
 * on-demand dan tidak bergantung pada sniffer.
 */
(() => {
  "use strict";

  const MEDIA_RE =
    /\.(m3u8|mpd|mp4|webm|mkv|m4v|mov|flv|wmv|mp3|m4a|aac|ogg|opus|flac)([?#].*)?$/i;

  // v2.10.5: MEDIA_RE menuntut ekstensi di akhir PATH, sehingga URL yang
  // menyimpan nama media di QUERY string terlewat — mis.
  //   https://cdn.test/download?file=video.mp4&token=…
  //   https://cdn.test/stream?src=master.m3u8
  // Cocokkan pula bila NILAI sebuah parameter query memuat ekstensi media.
  const MEDIA_QUERY_RE =
    /[?&][^&#]*=[^&#]*\.(m3u8|mpd|mp4|webm|mkv|m4v|mov|flv|wmv|mp3|m4a|aac|ogg|opus|flac)(?:[&#]|$)/i;

  function isMediaUrl(clean) {
    return MEDIA_RE.test(clean) || MEDIA_QUERY_RE.test(clean);
  }

  const candidates = new Set();
  const MAX = 50;

  // ── v2.9.4 (B4c): parameter gate "dormant" ──

  /// Jeda setelah event `load` sebelum halaman dianggap tanpa media.
  const IDLE_MS = 8000;
  /// Tag yang sendiri sudah merupakan sinyal media (pemeriksaan nodeName murah).
  const MEDIA_TAGS = new Set(["VIDEO", "AUDIO", "SOURCE"]);
  /// Selector untuk mencari sinyal media di dalam satu subtree yang baru added.
  const MEDIA_SELECTOR = "video, audio, source";

  // ── persist: tulis kandidat ke DOM (jembatan ke ISOLATED world) ──

  let persistScheduled = false;

  function persist() {
    if (persistScheduled) return;
    persistScheduled = true;
    // Microtask, BUKAN requestAnimationFrame: rAF di-throttle (bahkan
    // dihentikan) di tab latar sehingga kandidat bisa tidak pernah terbaca
    // oleh content.js. Microtask tetap menggabungkan seluruh `add()` yang
    // terjadi dalam satu blok sinkron menjadi SATU serialisasi.
    Promise.resolve().then(() => {
      persistScheduled = false;
      try {
        document.documentElement.dataset.fastdmMedia = JSON.stringify(
          Array.from(candidates),
        );
      } catch (e) {
        /* ignore */
      }
    });
  }

  function add(url) {
    try {
      if (!url || typeof url !== "string") return;
      // BUG FIX: XHR/fetch sering memakai URL RELATIF ("video.m3u8") —
      // resolve terhadap location agar tetap tertangkap.
      let abs = url;
      if (!/^https?:/i.test(abs)) {
        try {
          abs = new URL(url, location.href).href;
        } catch (e) {
          return; // tidak bisa di-resolve → abaikan
        }
      }
      if (!/^https?:/i.test(abs)) return; // abaikan blob:, data:, file:
      const clean = abs.split("#")[0];
      if (!isMediaUrl(clean)) return;
      // B4a: URL yang sudah tercatat tidak mengubah isi Set — keluar lebih
      // awal supaya tidak memicu serialisasi ulang yang sia-sia.
      if (candidates.has(clean)) return;
      candidates.add(clean);
      if (candidates.size > MAX) {
        const first = candidates.values().next().value;
        candidates.delete(first);
      }
      persist();
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

  // ── 3. Scan DOM ──

  /// Kumpulkan URL media dari satu elemen media.
  function collectFrom(el) {
    add(el.src);
    add(el.currentSrc);
  }

  /// Pindai SATU root (Document atau subtree yang baru ditambahkan) — bukan
  /// seluruh dokumen. B4b: biaya kini sebanding dengan ukuran konten baru,
  /// bukan ukuran halaman.
  function scanRoot(root) {
    if (!root) return;
    // root sendiri bisa elemen media (mis. <video> yang baru disisipkan).
    // nodeType 1 = ELEMENT; Document (9) tidak punya .src.
    if (root.nodeType === 1) collectFrom(root);
    if (!root.querySelectorAll) return;
    for (const el of root.querySelectorAll(MEDIA_SELECTOR)) {
      collectFrom(el);
    }
    for (const a of root.querySelectorAll("a[href]")) {
      add(a.href); // .href = absolut (property, bukan attribute)
    }
  }

  function scanAll() {
    scanRoot(document);
  }

  // ── v2.9.4 (B4b/c): antrian node + gate dormant ──

  let pending = new Set();
  let scanTimer = null;
  let dormant = false;
  let lastUrl = location.href;

  function flushScan() {
    scanTimer = null;
    if (dormant) return;
    // Tukar referensi dulu: scanRoot → add → (bisa memicu mutasi baru) tidak
    // boleh menulis ke batch yang sedang kita iterasi.
    const batch = pending;
    pending = new Set();
    for (const node of batch) scanRoot(node);
  }

  function scheduleScan() {
    if (scanTimer !== null) return;
    // 300 ms: menggabungkan burst mutasi SPA tanpa terasa lambat bagi user.
    scanTimer = setTimeout(flushScan, 300);
  }

  /// Apakah satu node yang baru ditambahkan merupakan sinyal media?
  /// Sengaja murah: diperiksa HANYA saat dormant, untuk memutuskan apakah
  /// perlu bangun — bukan untuk mengumpulkan URL.
  function looksLikeMedia(node) {
    if (node.nodeType !== 1) return false;
    if (MEDIA_TAGS.has(node.nodeName)) return true;
    if (node.nodeName === "A") {
      // Link media langsung: cocokkan href terhadap pola, jauh lebih murah
      // daripada bangun lalu menyapu subtree.
      const href = node.getAttribute && node.getAttribute("href");
      return !!href && isMediaUrl(href.split("#")[0]);
    }
    if (!node.querySelector) return false;
    // Subtree baru yang memuat player — short-circuit pada kecocokan pertama.
    return node.querySelector(MEDIA_SELECTOR) !== null;
  }

  function wake() {
    if (!dormant) return;
    dormant = false;
    // Halaman berubah bentuk sejak tidur — sapu ulang sekali agar media yang
    // muncul selama dormant tidak terlewat.
    scanAll();
    scheduleScan();
    // Boleh tidur lagi bila ternyata tetap tidak ada media.
    setTimeout(considerSleep, IDLE_MS);
  }

  function considerSleep() {
    if (dormant) return;
    // Tidur HANYA bila tidak ada satu pun sinyal media: tidak ada kandidat
    // hasil sniffing fetch/XHR DAN tidak ada elemen media di DOM.
    if (candidates.size > 0) return;
    if (document.querySelector(MEDIA_SELECTOR)) return;
    dormant = true;
  }

  function onMutate(mutations) {
    // Navigasi SPA mengganti konten tanpa memuat ulang dokumen.
    if (location.href !== lastUrl) {
      lastUrl = location.href;
      wake();
    }

    let wakeNeeded = false;
    for (const m of mutations) {
      if (m.type === "attributes") {
        // attributeFilter membatasi ini ke perubahan `src` saja.
        if (m.target && !dormant) pending.add(m.target);
        continue;
      }
      for (const node of m.addedNodes) {
        if (node.nodeType !== 1) continue;
        if (dormant) {
          if (looksLikeMedia(node)) wakeNeeded = true;
          continue;
        }
        pending.add(node);
      }
    }

    if (wakeNeeded) wake();
    if (!dormant && pending.size > 0) scheduleScan();
  }

  /// Pasang gate dormant: IDLE_MS setelah `load` (atau segera bila dokumen
  /// sudah lengkap — script bisa dievaluasi pada halaman yang sudah jadi).
  function armIdleGate() {
    if (document.readyState === "complete") {
      setTimeout(considerSleep, IDLE_MS);
      return;
    }
    window.addEventListener("load", () => setTimeout(considerSleep, IDLE_MS), {
      once: true,
    });
  }

  function start() {
    scanAll();
    new MutationObserver(onMutate).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["src"],
    });
    armIdleGate();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
