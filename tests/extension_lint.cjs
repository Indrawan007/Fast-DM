// Chromium-focused, dependency-free extension lint.
// `web-ext lint` targets Firefox and rejects MV3 service_worker manifests, so
// this checks the contract that Chrome/Brave/Edge actually load and then
// delegates JavaScript syntax validation to the same Node runtime as CI.
const { execFileSync } = require("node:child_process");
const { existsSync, readFileSync } = require("node:fs");
const { join } = require("node:path");

const root = join(__dirname, "..", "extension");
const manifestPath = join(root, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

function fail(message) {
  throw new Error(`[extension-lint] ${message}`);
}

if (manifest.manifest_version !== 3) fail("manifest_version harus 3");
if (
  !manifest.background ||
  manifest.background.service_worker !== "background.js"
) {
  fail("background.service_worker harus menunjuk background.js");
}
if (!existsSync(join(root, manifest.background.service_worker))) {
  fail("background service worker tidak ditemukan");
}
if (!Array.isArray(manifest.permissions) || manifest.permissions.length === 0) {
  fail("permissions harus berupa array tidak kosong");
}
if (
  !Array.isArray(manifest.host_permissions) ||
  !manifest.host_permissions.includes("<all_urls>")
) {
  fail("host_permissions harus mencakup <all_urls>");
}
if (!manifest.action || !manifest.action.default_popup) {
  fail("action.default_popup wajib ada");
}
if (!existsSync(join(root, manifest.action.default_popup))) {
  fail("popup yang dirujuk manifest tidak ditemukan");
}

for (const script of manifest.content_scripts || []) {
  if (!Array.isArray(script.js) || script.js.length === 0) {
    fail("setiap content script harus memiliki js");
  }
  for (const file of script.js) {
    if (!existsSync(join(root, file)))
      fail(`content script tidak ditemukan: ${file}`);
  }
}

for (const file of ["background.js", "content.js", "sniffer.js", "popup.js"]) {
  const path = join(root, file);
  if (!existsSync(path)) fail(`script wajib tidak ditemukan: ${file}`);
  execFileSync(process.execPath, ["--check", path], { stdio: "inherit" });
}

for (const [size, file] of Object.entries(
  manifest.action?.default_icon || {},
)) {
  if (!existsSync(join(root, file)))
    fail(`icon action ${size} tidak ditemukan: ${file}`);
}
for (const [size, file] of Object.entries(manifest.icons || {})) {
  if (!existsSync(join(root, file)))
    fail(`icon ${size} tidak ditemukan: ${file}`);
}

console.log("Chromium extension manifest and JavaScript lint passed");
