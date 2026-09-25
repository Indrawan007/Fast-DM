const NATIVE_HOST_NAME = "com.fastdm.native";

const DEFAULT_CONFIG = {
  enabled: true,
  interceptDownloads: true,
  interceptMinSize: 1048576,
  videoExtensions: [
    ".3g2",
    ".3ga",
    ".3gp",
    ".a52",
    ".aac",
    ".ac3",
    ".aif",
    ".aifc",
    ".aiff",
    ".alac",
    ".amr",
    ".amv",
    ".ape",
    ".asf",
    ".asx",
    ".au",
    ".aup",
    ".aup3",
    ".avi",
    ".awb",
    ".cda",
    ".dat",
    ".divx",
    ".dts",
    ".dv",
    ".f4v",
    ".flac",
    ".flc",
    ".fli",
    ".flv",
    ".hdv",
    ".m2ts",
    ".m2v",
    ".m3u8",
    ".m4a",
    ".m4b",
    ".m4r",
    ".m4v",
    ".mid",
    ".midi",
    ".mka",
    ".mkv",
    ".mod",
    ".mov",
    ".mp1",
    ".mp2",
    ".mp2v",
    ".mp3",
    ".mp4",
    ".mpc",
    ".mpd",
    ".mpe",
    ".mpeg",
    ".mpg",
    ".mpp",
    ".mts",
    ".mxf",
    ".nsv",
    ".ogg",
    ".ogm",
    ".ogv",
    ".ogx",
    ".opus",
    ".qt",
    ".ra",
    ".rm",
    ".rmvb",
    ".roq",
    ".shn",
    ".tak",
    ".tod",
    ".ts",
    ".tta",
    ".vob",
    ".vro",
    ".wav",
    ".wavpack",
    ".weba",
    ".webm",
    ".wma",
    ".wmv",
    ".wmx",
    ".wv",
    ".wvx",
    ".xvid",
    ".yuv",
  ],
  fileExtensions: [
    ".3ds",
    ".3mf",
    ".7z",
    ".7zip",
    ".a",
    ".abw",
    ".ace",
    ".afdesign",
    ".afphoto",
    ".ai",
    ".alz",
    ".apk",
    ".apks",
    ".appimage",
    ".appx",
    ".appxbundle",
    ".ar",
    ".arc",
    ".arj",
    ".arw",
    ".ass",
    ".avif",
    ".azw",
    ".azw3",
    ".backup",
    ".bak",
    ".bat",
    ".bib",
    ".bin",
    ".blend",
    ".bmp",
    ".bson",
    ".bundle",
    ".bz",
    ".bz2",
    ".cab",
    ".cdr",
    ".chm",
    ".cmd",
    ".com",
    ".cr2",
    ".crx",
    ".csv",
    ".cue",
    ".dae",
    ".db",
    ".db3",
    ".deb",
    ".djv",
    ".djvu",
    ".dmg",
    ".dng",
    ".doc",
    ".docx",
    ".dump",
    ".dwg",
    ".dxf",
    ".ear",
    ".emf",
    ".eot",
    ".eps",
    ".epub",
    ".esd",
    ".exe",
    ".fb2",
    ".fbx",
    ".fbz",
    ".flatpak",
    ".fnt",
    ".fon",
    ".gadget",
    ".gif",
    ".glb",
    ".gltf",
    ".gz",
    ".heic",
    ".heif",
    ".ico",
    ".ics",
    ".iges",
    ".igs",
    ".img",
    ".indd",
    ".ipa",
    ".iso",
    ".j2c",
    ".j2k",
    ".jar",
    ".jp2",
    ".jpeg",
    ".jpf",
    ".jpg",
    ".jpm",
    ".jpx",
    ".json",
    ".jxl",
    ".jxr",
    ".key",
    ".kra",
    ".lha",
    ".lit",
    ".log",
    ".lrf",
    ".lz",
    ".lz4",
    ".lzh",
    ".lzma",
    ".lzo",
    ".markdown",
    ".md",
    ".meta4",
    ".metalink",
    ".mobi",
    ".msi",
    ".msix",
    ".msixbundle",
    ".msu",
    ".nef",
    ".nfo",
    ".numbers",
    ".nzb",
    ".obj",
    ".odf",
    ".odg",
    ".odp",
    ".ods",
    ".odt",
    ".off",
    ".orf",
    ".otf",
    ".ova",
    ".ovf",
    ".oxps",
    ".pages",
    ".pak",
    ".pdb",
    ".pdf",
    ".pkg",
    ".ply",
    ".pml",
    ".png",
    ".ppt",
    ".pptx",
    ".psd",
    ".qcow2",
    ".rar",
    ".raw",
    ".rb",
    ".rpm",
    ".rtf",
    ".run",
    ".s7z",
    ".sh",
    ".sldasm",
    ".sldprt",
    ".snap",
    ".sql",
    ".sqlite",
    ".sqlite3",
    ".sr2",
    ".srt",
    ".ssa",
    ".step",
    ".stl",
    ".stp",
    ".sub",
    ".svg",
    ".svgz",
    ".swm",
    ".tar",
    ".tbz",
    ".tbz2",
    ".tcr",
    ".tex",
    ".tgz",
    ".tif",
    ".tiff",
    ".tlz",
    ".torrent",
    ".ttc",
    ".ttf",
    ".txt",
    ".txtz",
    ".txz",
    ".vcf",
    ".vdi",
    ".vhd",
    ".vhdx",
    ".vmdk",
    ".vtt",
    ".war",
    ".webp",
    ".wim",
    ".wmf",
    ".woff",
    ".woff2",
    ".wsf",
    ".x3d",
    ".xapk",
    ".xcf",
    ".xls",
    ".xlsx",
    ".xml",
    ".xpi",
    ".xps",
    ".xz",
    ".yaml",
    ".yml",
    ".z",
    ".zip",
    ".zipx",
    ".zst",
    ".zz",
  ],
  excludePatterns: [],
};

let config = { ...DEFAULT_CONFIG };

// B4e: config pindah dari storage.sync ke storage.local. Config ini
// per-mesin (intersep, ambang ukuran, daftar ekstensi file) — bukan preferensi
// yang perlu ikut ke perangkat lain, sedangkan `sync` punya kuota ketat
// (8 KB/item, 512 tulis/hari, throttle) yang bisa membuat `setConfig` gagal
// tanpa terlihat oleh user (L6).
chrome.storage.local.get("config", (result) => {
  if (result.config) {
    config = { ...DEFAULT_CONFIG, ...result.config };
    return;
  }
  // Migrasi satu arah dari ≤2.9.3. Bila tulis ke local gagal, salinan di sync
  // dibiarkan utuh supaya config lama tidak hilang.
  chrome.storage.sync.get("config", (old) => {
    if (!old.config) return;
    config = { ...DEFAULT_CONFIG, ...old.config };
    chrome.storage.local.set({ config }, () => {
      if (chrome.runtime.lastError) return;
      chrome.storage.sync.remove("config");
    });
  });
});
chrome.storage.onChanged.addListener((changes, area) => {
  // `registered_id` juga hidup di local — batasi ke perubahan config saja.
  if (area === "local" && changes.config)
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
      title: "⚡ Unduh dengan Fast DM",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-video",
      title: "⚡ Unduh Video dengan Fast DM",
      contexts: ["video", "audio"],
    });
    chrome.contextMenus.create({
      id: "fastdm-download-image",
      title: "⚡ Unduh Gambar dengan Fast DM",
      contexts: ["image"],
    });
  });
});

// Register ulang saat browser restart (deduplikasi internal:
// registerExtensionId cek storage & di-skip bila ID sudah terdaftar)
chrome.runtime.onStartup.addListener(() => {
  registerExtensionId();
});

// ═══════════════════════════════════════════════
// Native Messaging
// ═══════════════════════════════════════════════

// v2.3.0 (L7): batas waktu eksplisit. Native host bisa menahan request cukup
// lama saat cold start GUI (poll socket hingga ±15 dtk + forward 5 dtk);
// tanpa timeout, promise menggantung tanpa umpan balik ke user.
const NATIVE_TIMEOUT_MS = 25_000;

function sendToNative(message) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let timer;
    const finish = (fn, arg) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      fn(arg);
    };
    timer = setTimeout(
      () =>
        finish(
          reject,
          new Error("Timeout — Fast DM tidak merespons (apakah baru mulai?)"),
        ),
      NATIVE_TIMEOUT_MS,
    );
    try {
      chrome.runtime.sendNativeMessage(NATIVE_HOST_NAME, message, (resp) => {
        if (chrome.runtime.lastError) {
          finish(reject, chrome.runtime.lastError);
          return;
        }
        finish(resolve, resp);
      });
    } catch (err) {
      finish(reject, err);
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
      for (const key of ["filename", "file", "name", "title", "fn"]) {
        const val = urlObj.searchParams.get(key);
        if (val && val.includes(".")) {
          filename = decodeURIComponent(val);
          break;
        }
      }
      if (!filename) {
        const path = decodeURIComponent(urlObj.pathname);
        const parts = path.split("/").filter(Boolean);
        if (parts.length > 0) {
          const last = parts[parts.length - 1];
          if (last.includes(".")) filename = last;
        }
      }
    } catch (e) {
      /* ignore */
    }
  }

  // Keep the browser identity alongside its cookies. Some sessions are tied
  // to User-Agent; the downloader's hardcoded fallback may not match it.
  // Clone so callers' headers stay unchanged; preserve explicit UA overrides.
  const requestHeaders = { ...headers };
  if (
    !Object.keys(requestHeaders).some(
      (key) => key.toLowerCase() === "user-agent",
    ) &&
    typeof navigator !== "undefined" &&
    navigator.userAgent
  ) {
    requestHeaders["User-Agent"] = navigator.userAgent;
  }

  const message = {
    action: "download",
    url: url,
    filename: filename,
    headers: requestHeaders,
    extension_id: chrome.runtime.id,
  };

  // Tambah quality jika ada (untuk YouTube)
  if (quality) {
    message.quality = quality;
  }

  // Ambil cookies situs via API bila tidak dikirim eksplisit.
  // Kirim metadata cookie utuh, bukan hanya string "name=value": downloader
  // perlu mempertahankan host-only/domain, path, Secure, dan expiry agar
  // cookie HTTPS tidak pernah turun ke HTTP atau sibling subdomain.
  if (!cookies) {
    try {
      const jar = await chrome.cookies.getAll({ url });
      if (Array.isArray(jar)) {
        cookies = jar;
        domain = new URL(url).hostname;
      }
    } catch (e) {
      /* ignore */
    }
  }

  // Cookies halaman (untuk yt-dlp/aria2 — download login-protected).
  // `domain` dipertahankan untuk kompatibilitas message lama, tetapi Rust
  // memvalidasi ulang terhadap URL request dan metadata setiap cookie.
  if (Array.isArray(cookies) && domain) {
    // Kirim array kosong juga: native host perlu menghapus jar lama agar
    // kredensial dari unduhan sebelumnya tidak dipakai ulang diam-diam.
    message.cookies = cookies;
    message.domain = domain;
  }

  // B4f: badge pending — sendToNative menunggu hingga 25 detik sebelum
  // timeout. Tanpa penanda ini user tidak dapat umpan balik sama sekali saat
  // native host lambat atau tidak merespons. `holdMs = 0` menahannya sampai
  // badge hasil (⬇/!) menggantikannya.
  showBadge("…", "#a6adc8", 0);

  try {
    const response = await sendToNative(message);
    if (!response || !response.success) {
      showBadge("!", "#f38ba8");
      return (
        response || {
          success: false,
          error: "Native host tidak memberi respons",
        }
      );
    }
    // URL bisa mengandung token; cukup log status, bukan URL/nama file.
    console.log("[FastDM] Download accepted");
    showBadge("⬇", "#89b4fa");
    return response;
  } catch (err) {
    console.error("[FastDM] Failed:", err);
    showBadge("!", "#f38ba8");
    return null;
  }
}

// B4f: timer badge dilacak. Tanpa ini, urutan "…" → "⬇" (atau dua unduhan
// berdekatan) membuat timeout milik badge LAMA menghapus badge yang baru
// dipasang sebelum 3 detik habis.
//
// L6: setTimeout saja tidak cukup untuk MV3 — service worker boleh disuspend
// sebelum callback berjalan. Timer lokal tetap dipakai agar badge hilang tepat
// waktu saat worker aktif, sedangkan chrome.alarms menjadi fallback persisten
// yang membangunkan worker setelah suspend.
const BADGE_CLEAR_ALARM = "fastdm-badge-clear";
let badgeTimer = null;
let badgeGeneration = 0;

function clearBadgeAlarm() {
  if (chrome.alarms?.clear) chrome.alarms.clear(BADGE_CLEAR_ALARM);
}

function showBadge(text, color, holdMs = 3000) {
  const generation = ++badgeGeneration;
  if (badgeTimer) clearTimeout(badgeTimer);
  chrome.action.setBadgeText({ text });
  chrome.action.setBadgeBackgroundColor({ color });

  if (holdMs <= 0) {
    badgeTimer = null;
    clearBadgeAlarm();
    return;
  }

  // create() dengan nama yang sama mengganti alarm sebelumnya. Jangan
  // mengandalkan clear() lalu create() berurutan karena keduanya async.
  if (chrome.alarms?.create) {
    chrome.alarms.create(BADGE_CLEAR_ALARM, {
      when: Date.now() + holdMs,
    });
  }
  badgeTimer = setTimeout(() => {
    if (generation !== badgeGeneration) return;
    badgeTimer = null;
    chrome.action.setBadgeText({ text: "" });
    clearBadgeAlarm();
  }, holdMs);
}

if (chrome.alarms?.onAlarm) {
  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== BADGE_CLEAR_ALARM) return;
    // Alarm dapat membangunkan worker baru; pada worker lama invalidasi timer
    // lokal agar callback yang terlambat tidak menghapus badge berikutnya.
    badgeGeneration += 1;
    if (badgeTimer) clearTimeout(badgeTimer);
    badgeTimer = null;
    chrome.action.setBadgeText({ text: "" });
  });
}

// ═══════════════════════════════════════════════
// Download Interception
// ═══════════════════════════════════════════════

// URL yang di-restart ulang oleh fallback kita sendiri. Tanpa ini, download
// fallback akan ter-intercept lagi → cancel → fallback lagi → loop tak
// berujung saat native host tidak tersedia.
//
// v2.9.3: entri kedaluwarsa otomatis. Dulu entri hanya dihapus saat event
// onCreated untuk URL yang sama tiba — kalau download fallback tidak pernah
// terbentuk (user menutup dialog "Simpan sebagai", URL ditolak Chrome, dst.)
// entri tertinggal selamanya dan menumpuk di service worker; unduhan ULANG
// URL yang sama juga jadi ikut dilewatkan (tidak dikirim ke Fast DM).
const SELF_INITIATED_TTL_MS = 60_000;
const selfInitiated = new Map();

function markSelfInitiated(url) {
  selfInitiated.set(url, Date.now() + SELF_INITIATED_TTL_MS);
}

/** true = event ini berasal dari fallback kita sendiri (sekali pakai). */
function consumeSelfInitiated(url) {
  const now = Date.now();
  // Sapu entri kedaluwarsa sekalian — jumlahnya kecil, biayanya sepele.
  for (const [key, expiry] of selfInitiated) {
    if (expiry <= now) selfInitiated.delete(key);
  }
  if (!selfInitiated.has(url)) return false;
  selfInitiated.delete(url);
  return true;
}

function isSelfInitiated(url) {
  const now = Date.now();
  for (const [key, expiry] of selfInitiated) {
    if (expiry <= now) selfInitiated.delete(key);
  }
  return selfInitiated.has(url);
}

const handledUrls = new Map();

function markHandled(url) {
  handledUrls.set(url, Date.now() + SELF_INITIATED_TTL_MS);
}

function wasHandled(url) {
  const now = Date.now();
  for (const [key, expiry] of handledUrls) {
    if (expiry <= now) handledUrls.delete(key);
  }
  return handledUrls.has(url);
}

// v3.2.3 (A6): callback API Chrome yang mengabaikan `chrome.runtime.lastError`
// membuat Chrome menulis "Unchecked runtime.lastError" ke console service
// worker. Semua callback best-effort (cancel/erase/download) lewat sini.
function noteRuntimeError(context) {
  if (chrome.runtime.lastError) {
    console.debug("[FastDM]", context, chrome.runtime.lastError.message);
  }
}

// v3.2.3 (A6): unduhan yang kita batalkan dihapus dari shelf Chrome lewat
// `downloads.onChanged`, BUKAN di dalam callback `downloads.cancel`. Saat
// callback cancel berjalan, item masih berstatus `in_progress` dan
// `downloads.erase` menolaknya — entri "dibatalkan" tertinggal di shelf.
// `erase` sekarang menunggu status benar-benar `interrupted`.
const cancelThenErase = new Set();

function cancelAndForget(id) {
  cancelThenErase.add(id);
  chrome.downloads.cancel(id, () => noteRuntimeError("cancel"));
}

chrome.downloads.onChanged.addListener((delta) => {
  if (!cancelThenErase.has(delta.id)) return;
  const state = delta.state?.current;
  // Lepaskan entri pada status terminal APA PUN (bukan hanya `interrupted`)
  // supaya `cancelThenErase` tidak menumpuk bila unduhan berakhir sebagai
  // `complete` (mis. bytes-nya sudah habis saat cancel diproses).
  if (state !== "interrupted" && state !== "complete") return;
  cancelThenErase.delete(delta.id);
  if (state !== "interrupted") return;
  chrome.downloads.erase({ id: delta.id }, () => noteRuntimeError("erase"));
});

function basename(path) {
  if (!path) return null;
  const parts = path.replace(/\\/g, "/").split("/");
  const last = parts[parts.length - 1];
  return last && last.includes(".") ? last : null;
}

function filenameHasInterceptedExt(filename) {
  if (!filename) return false;
  const lower = filename.toLowerCase();
  const allExts = [...config.videoExtensions, ...config.fileExtensions];
  return allExts.some((ext) => lower.endsWith(ext));
}

// v3.2.7: `onCreated` BUKAN hanya untuk unduhan baru. Saat browser start,
// Chrome memuat riwayat unduhan dari disk dan memancarkan `onCreated` untuk
// SETIAP entri lama — state-nya `complete` atau `interrupted`, bukan
// `in_progress`. Tanpa guard ini entri riwayat diperlakukan seperti unduhan
// baru: cancel (gagal, item sudah terminal → `onChanged` tidak pernah datang →
// erase tidak pernah dipanggil → entri tetap ada), lalu `sendDownload` →
// native host tidak menemukan socket → menyalakan GUI → URL lama diunduh
// ulang. Diulang pada SETIAP start browser karena entrinya tidak pernah
// hilang. Entri yang tertinggal berasal dari v3.2.2 (erase di dalam callback
// cancel ditolak Chrome) dan dari jalur fallback `saveAs`.
function isLiveDownload(downloadItem) {
  return downloadItem.state === "in_progress";
}

// v3.3.2: serahkan kembali ke browser bila server MENOLAK Fast-DM.
// Unduhan yang kita cegat sudah dibatalkan di browser, lalu diterima Fast-DM
// (`success: true`) — tetapi hasil sebenarnya baru diketahui setelah aria2
// menghubungi server. File-host dengan anti-bot/hotlink-protection, sesi yang
// terikat sidik jari TLS browser, atau tautan bertanda tangan sering menjawab
// HTTP 403 (aria2 exit 22) atau halaman HTML hanya untuk Fast-DM. Dulu unduhan
// itu berakhir "GAGAL" dan user kehilangan unduhannya sama sekali. Sekarang
// extension bertanya ke Fast-DM (aksi `handback`) sampai hasilnya jelas; bila
// ditolak, browser mengunduhnya sendiri seperti biasa.
const HANDBACK_POLL_MS = 2_000;
const HANDBACK_MAX_MS = 120_000;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function watchForHandback(id, url) {
  if (typeof id !== "string" || !id) return false;
  const deadline = Date.now() + HANDBACK_MAX_MS;
  while (Date.now() < deadline) {
    await sleep(HANDBACK_POLL_MS);
    let resp;
    try {
      resp = await sendToNative({ action: "handback", id });
    } catch (e) {
      return false; // Fast-DM ditutup / host tak tersedia — jangan tebak.
    }
    if (!resp || !resp.success) return false;
    if (resp.message === "pending") continue;
    if (resp.message !== "handback") return false;
    // Fast-DM sudah menandai itemnya "Diserahkan ke browser" secara atomik;
    // unduhan berikut berasal dari kita sendiri dan tidak boleh dicegat lagi.
    // Nama file dibiarkan ditentukan browser (Content-Disposition asli).
    markSelfInitiated(url);
    chrome.downloads.download({ url }, () => noteRuntimeError("handback"));
    showBadge("↩", "#f9e2af");
    return true;
  }
  return false;
}

chrome.downloads.onCreated.addListener(async (downloadItem) => {
  if (!config.enabled || !config.interceptDownloads) return;

  if (!isLiveDownload(downloadItem)) return;

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return;

  if (consumeSelfInitiated(url)) {
    markHandled(url);
    return;
  }

  if (wasHandled(url)) return;

  if (!shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime))
    return;

  markHandled(url);

  cancelAndForget(downloadItem.id);

  const filename = basename(downloadItem.filename);

  const headers = {};
  if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

  const result = await sendDownload(url, filename, headers).catch(() => null);
  if (result && result.success) {
    void watchForHandback(result.id, url);
  } else {
    // Fallback: restart download in Chrome normally.
    // v3.2.3 (A7): `saveAs: true` saja — `filename` diabaikan Chrome saat
    // dialog "Simpan sebagai" terbuka, jadi mengirim keduanya hanya menyesatkan.
    markSelfInitiated(url);
    chrome.downloads.download({ url, saveAs: true }, () =>
      noteRuntimeError("download"),
    );
  }
});

chrome.downloads.onDeterminingFilename.addListener((downloadItem, suggest) => {
  const keep = () => suggest({ filename: downloadItem.filename });

  if (!config.enabled || !config.interceptDownloads) return keep();

  const url = downloadItem.finalUrl || downloadItem.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:")) return keep();

  if (isSelfInitiated(url) || wasHandled(url)) return keep();

  const filename = basename(downloadItem.filename);
  const intercept =
    shouldInterceptUrl(url, downloadItem.fileSize, downloadItem.mime) ||
    filenameHasInterceptedExt(filename);

  if (!intercept) return keep();

  markHandled(url);
  cancelAndForget(downloadItem.id);

  const headers = {};
  if (downloadItem.referrer) headers["Referer"] = downloadItem.referrer;

  sendDownload(url, filename, headers).then((result) => {
    if (result && result.success) {
      void watchForHandback(result.id, url);
    } else {
      // v3.2.3 (A7): lihat catatan di `onCreated` — `saveAs: true` saja.
      markSelfInitiated(url);
      chrome.downloads.download({ url, saveAs: true }, () =>
        noteRuntimeError("download"),
      );
    }
  });

  keep();
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

function sniffedCandidatesForTab(tabId) {
  if (!tabId || !chrome.tabs?.sendMessage) return Promise.resolve([]);
  return new Promise((resolve) => {
    try {
      chrome.tabs.sendMessage(
        tabId,
        { action: "getSniffedCandidates" },
        (response) => {
          if (chrome.runtime.lastError) {
            resolve([]);
            return;
          }
          const urls =
            response && Array.isArray(response.urls) ? response.urls : [];
          resolve(
            urls.filter(
              (candidate) =>
                typeof candidate === "string" &&
                candidate.length > 0 &&
                !candidate.startsWith("blob:") &&
                !candidate.startsWith("mediastream:"),
            ),
          );
        },
      );
    } catch (e) {
      // Restricted pages may not have a content script; retain normal fallback.
      resolve([]);
    }
  });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  let url = null;
  let filename = null;

  // v3.2.2: `videoMenu` dulu dipakai di bawah tanpa pernah dideklarasikan,
  // sehingga handler async ini melempar ReferenceError untuk SEMUA item menu
  // (link/video/gambar) — bukan hanya kasus blob:. Ditetapkan eksplisit dari
  // menuItemId supaya cabang sniffer hanya jalan untuk menu video.
  const videoMenu = info.menuItemId === "fastdm-download-video";

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

  // A video element backed by MSE normally exposes only blob: in the context
  // menu. Ask the content script for URLs captured by the MAIN-world sniffer
  // instead of sending an unusable blob URL to aria2/yt-dlp.
  if (
    videoMenu &&
    (!url || url.startsWith("blob:") || url.startsWith("mediastream:"))
  ) {
    const candidates = await sniffedCandidatesForTab(tab?.id);
    url = candidates.at(-1) || null;
    if (!url) {
      console.warn("[FastDM] No downloadable media candidate for context menu");
      showBadge("!", "#f38ba8");
      return;
    }
  }

  if (!url) return;

  const headers = {};
  if (info.pageUrl) headers["Referer"] = info.pageUrl;

  try {
    const urlObj = new URL(url);
    for (const key of ["filename", "file", "name", "title", "fn"]) {
      const val = urlObj.searchParams.get(key);
      if (val && val.includes(".")) {
        filename = decodeURIComponent(val);
        break;
      }
    }
    if (!filename) {
      const path = urlObj.pathname;
      const decoded = decodeURIComponent(path);
      const parts = decoded.split("/").filter(Boolean);
      if (parts.length > 0) {
        const last = parts[parts.length - 1];
        if (last && last.includes(".")) filename = last;
      }
    }
  } catch (e) {
    /* ignore */
  }

  await sendDownload(url, filename, headers);
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
      // B14: sendDownload tidak pernah me-reject (internal catch → null),
      // jadi terjemahkan null jadi respons gagal yang eksplisit.
      .then((r) =>
        sendResponse(
          r || { success: false, error: "Gagal mengirim ke native host" },
        ),
      )
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
  if (message.action === "getConfig") {
    sendResponse(config);
    return false;
  }
  if (message.action === "setConfig") {
    config = { ...config, ...message.config };
    chrome.storage.local.set({ config });
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
