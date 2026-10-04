"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const ROOT = path.resolve(__dirname, "..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");

test("PKGBUILD valid dan skrip packaging lolos guard shell dasar", () => {
  const syntax = spawnSync("bash", ["-n", "packaging/PKGBUILD"], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(syntax.status, 0, syntax.stderr || syntax.stdout);

  const pkgbuild = read("packaging/PKGBUILD");
  assert.match(pkgbuild, /\)\nmakedepends=\('cargo' 'rust'\)/);
  assert.doesNotMatch(pkgbuild, /\)makedepends=/);
  assert.match(pkgbuild, /cd "\$srcdir\/\$pkgname-\$pkgver" \|\| return 1/g);

  const arch = read("packaging/build-arch.sh");
  const archSyntax = spawnSync("bash", ["-n", "packaging/build-arch.sh"], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(archSyntax.status, 0, archSyntax.stderr || archSyntax.stdout);
  assert.match(arch, /--exclude=\.\/node_modules/);
  assert.doesNotMatch(arch, /PKGFILE=\$\(ls\b/);
});

test("paket hanya menargetkan Arch + Hyprland/Wayland", () => {
  const pkgbuild = read("packaging/PKGBUILD");
  const desktop = read("packaging/fast-dm.desktop");
  const app = read("src/app.rs");
  const main = read("src/main.rs");
  const window = read("src/gui/window.rs");

  assert.match(pkgbuild, /wl-clipboard/);
  assert.match(pkgbuild, /Hyprland|Wayland/i);
  assert.doesNotMatch(pkgbuild, /xclip|X11|\.deb|Debian/i);
  assert.match(desktop, /GDK_BACKEND=wayland/);
  assert.match(app, /WAYLAND_DISPLAY/);
  assert.match(app, /GDK_BACKEND/);
  assert.match(main, /set_var\("GDK_BACKEND", backend\)/);
  assert.match(window, /wl-paste/);
  assert.doesNotMatch(window, /xclip|X11/);
  assert.equal(fs.existsSync(path.join(ROOT, "packaging/build-deb.sh")), false);
  assert.equal(fs.existsSync(path.join(ROOT, "packaging/control")), false);
});

test("CI dan release menjalankan build di container Arch saja", () => {
  const ci = read(".github/workflows/ci.yml");
  const release = read(".github/workflows/release.yml");
  const workflows = `${ci}\n${release}`;

  assert.match(workflows, /image:\s*archlinux:latest/);
  assert.match(workflows, /packaging\/build-arch\.sh/);
  assert.doesNotMatch(workflows, /apt-get|dpkg-deb|build-deb\.sh|\.deb/);
});

test("reqwest hanya mengaktifkan backend rustls", () => {
  const cargo = read("Cargo.toml");
  const start = cargo.indexOf("reqwest =");
  const end = cargo.indexOf("\n\n# URL parsing", start);
  assert.ok(
    start >= 0 && end > start,
    "blok dependency reqwest tidak ditemukan",
  );
  const reqwest = cargo.slice(start, end);
  assert.match(reqwest, /default-features\s*=\s*false/);
  assert.match(reqwest, /"rustls-tls"/);
  assert.match(reqwest, /"charset"/);
  assert.match(reqwest, /"http2"/);
  assert.match(reqwest, /"system-proxy"/);
  assert.doesNotMatch(reqwest, /"native-tls"/);

  const lock = read("Cargo.lock");
  for (const nativeTlsPackage of [
    "hyper-tls",
    "native-tls",
    "openssl-sys",
    "tokio-native-tls",
  ]) {
    assert.doesNotMatch(
      lock,
      new RegExp(`name = "${nativeTlsPackage}"`),
      `${nativeTlsPackage} tidak boleh tersisa di dependency graph`,
    );
  }
});

test("workflow Arch menyediakan tool Node dan artifact memakai action terbaru", () => {
  const ci = read(".github/workflows/ci.yml");
  const release = read(".github/workflows/release.yml");
  const workflows = `${ci}\n${release}`;

  assert.doesNotMatch(workflows, /actions\/checkout@v4/);
  assert.doesNotMatch(workflows, /actions\/upload-artifact@v[45]/);
  assert.match(workflows, /actions\/checkout@v5/);
  assert.match(ci, /actions\/upload-artifact@v6/);

  assert.match(ci, /nodejs/);
  assert.match(release, /nodejs/);
  assert.match(ci, /npm ci/);
  assert.match(release, /npm ci/);
});

test("workflow rilis menjalankan seluruh security gate sebelum publish", () => {
  const release = read(".github/workflows/release.yml");
  const publishAt = release.indexOf("Create GitHub Release");
  assert.ok(publishAt > 0, "langkah publish release tidak ditemukan");
  const gates = release.slice(0, publishAt);

  for (const command of [
    "cargo fmt --all -- --check",
    "cargo audit",
    "cargo deny check advisories bans licenses sources",
    "cargo clippy --all-targets -- -D warnings",
    "cargo test --locked",
    "node tools/check-undeclared.cjs",
  ]) {
    assert.ok(
      gates.includes(command),
      `${command} harus berjalan sebelum publish`,
    );
  }
});

test("cargo-deny mengizinkan lisensi webpki-roots", () => {
  assert.match(read("deny.toml"), /"CDLA-Permissive-2\.0"/);
});
