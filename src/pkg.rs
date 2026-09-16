//! Deteksi package manager distro — dipakai pesan "tool tidak terinstall".
//!
//! Pesan lama meng-hardcode `sudo apt install …`. Itu benar di Debian/Ubuntu
//! tapi menyesatkan di Arch (`pacman -S`), Fedora (`dnf install`), openSUSE
//! (`zypper install`), Void (`xbps-install`), dan Alpine (`apk add`) — user
//! Arch yang menjalankan perintah itu mendapat "command not found" tepat saat
//! ia sedang butuh aria2c.
//!
//! Keputusan intinya adalah **fungsi murni** (`PackageManager::from_ids` dan
//! `from_binaries`) supaya bisa di-unit test tanpa filesystem; pembungkusnya
//! (`detect`) hanya membaca `/etc/os-release` lalu mengecek keberadaan binary.
//! Deteksi hanya berjalan di jalur error (tool hilang), jadi tidak perlu cache
//! — dan sengaja tidak memakai crate baru (AGENTS.md §3).

/// Package manager yang dikenali. `Unknown` = tidak bisa memastikan; pemakai
/// tetap mendapat nama paket yang harus dipasang, tanpa perintah yang salah.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    /// Arch Linux & turunannya (Manjaro, EndeavourOS, Garuda, CachyOS, Archarm)
    Pacman,
    /// Debian & turunannya (Ubuntu, Mint, Pop!_OS, elementary, Zorin, Kali)
    Apt,
    /// Fedora / RHEL ≥ 8 / Rocky / AlmaLinux
    Dnf,
    /// openSUSE (Leap & Tumbleweed)
    Zypper,
    /// Void Linux
    Xbps,
    /// Alpine
    Apk,
    Unknown,
}

impl PackageManager {
    /// Perintah install satu paket, siap tempel di terminal.
    pub fn install_cmd(self, pkg: &str) -> String {
        match self {
            // Nama paket `aria2` & `yt-dlp` sama di semua distro yang didukung
            // (paket aria2 menyediakan binary `aria2c`; yt-dlp → `yt-dlp`).
            Self::Pacman => format!("sudo pacman -S {pkg}"),
            Self::Apt => format!("sudo apt install {pkg}"),
            Self::Dnf => format!("sudo dnf install {pkg}"),
            Self::Zypper => format!("sudo zypper install {pkg}"),
            Self::Xbps => format!("sudo xbps-install -S {pkg}"),
            Self::Apk => format!("sudo apk add {pkg}"),
            Self::Unknown => format!("install paket '{pkg}' lewat package manager distro Anda"),
        }
    }

    /// Pesan siap-tampil untuk binary downloader yang tidak ditemukan.
    /// `binary` = nama executable (`aria2c`), `pkg` = nama paket (`aria2`);
    /// keduanya berbeda untuk aria2, jadi tidak boleh disatukan.
    pub fn missing_tool_msg(self, binary: &str, pkg: &str) -> String {
        format!(
            "{binary} tidak terinstall — jalankan: {}",
            self.install_cmd(pkg)
        )
    }

    /// Inti murni: token `ID` / `ID_LIKE` dari `/etc/os-release` → manager.
    ///
    /// `ID_LIKE` ikut diperiksa karena banyak turunan tidak menyebut induknya
    /// di `ID` — Manjaro menulis `ID=manjaro` + `ID_LIKE=arch`, EndeavourOS
    /// `ID=endeavouros` + `ID_LIKE=arch`, Linux Mint `ID=linuxmint` +
    /// `ID_LIKE="ubuntu debian"`.
    pub fn from_ids(ids: &[&str]) -> Self {
        for id in ids {
            let id = id.trim().to_ascii_lowercase();
            let found = match id.as_str() {
                "arch" | "manjaro" | "endeavouros" | "garuda" | "cachyos" | "archarm"
                | "archlabs" => Self::Pacman,
                "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
                | "raspbian" | "deepin" | "mx" => Self::Apt,
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "nobara" => Self::Dnf,
                "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" | "sled" | "suse" => {
                    Self::Zypper
                }
                "void" => Self::Xbps,
                "alpine" => Self::Apk,
                _ => continue,
            };
            return found;
        }
        Self::Unknown
    }

    /// Inti murni: dari daftar binary yang **ada** di sistem → manager.
    /// Fallback bila `/etc/os-release` tidak ada/tidak dikenali (container
    /// minimal, distro rolling baru). Urutan input = prioritas pemanggil.
    pub fn from_binaries(present: &[&str]) -> Self {
        for bin in present {
            let found = match *bin {
                "pacman" => Self::Pacman,
                "apt-get" => Self::Apt,
                "dnf" => Self::Dnf,
                "zypper" => Self::Zypper,
                "xbps-install" => Self::Xbps,
                "apk" => Self::Apk,
                _ => continue,
            };
            return found;
        }
        Self::Unknown
    }
}

/// Ambil token `ID` dan `ID_LIKE` dari isi `/etc/os-release`.
///
/// Aturan spesifikasi yang dipakai: nilai boleh dikutip
/// (`ID_LIKE="ubuntu debian"`), baris `#` adalah komentar, kunci lain
/// (`NAME`, `PRETTY_NAME`, …) diabaikan. Hasil lowercase + tanpa kutip.
pub fn parse_os_release(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !matches!(key.trim(), "ID" | "ID_LIKE") {
            continue;
        }
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        for token in value.split_whitespace() {
            let token = token.to_ascii_lowercase();
            if !token.is_empty() && !out.contains(&token) {
                out.push(token);
            }
        }
    }
    out
}

/// Deteksi package manager sistem ini: `/etc/os-release` dulu, lalu keberadaan
/// binary. Tidak pernah panic — kegagalan I/O apa pun berarti "tidak tahu".
pub fn detect() -> PackageManager {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let ids = parse_os_release(&content);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let pm = PackageManager::from_ids(&refs);
        if pm != PackageManager::Unknown {
            return pm;
        }
    }
    const CANDIDATES: &[&str] = &[
        "/usr/bin/pacman",
        "/usr/bin/apt-get",
        "/usr/bin/dnf",
        "/usr/bin/zypper",
        "/usr/bin/xbps-install",
        "/usr/sbin/apk",
    ];
    let present: Vec<&str> = CANDIDATES
        .iter()
        .filter(|p| std::path::Path::new(p).exists())
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        })
        .collect();
    PackageManager::from_binaries(&present)
}

/// Perintah install `pkg` di distro ini (mis. `"aria2"` →
/// `"sudo pacman -S aria2"` di Arch).
pub fn install_hint(pkg: &str) -> String {
    detect().install_cmd(pkg)
}

/// Pesan siap-tampil untuk binary downloader yang hilang, dengan deteksi
/// distro otomatis. Dipakai `aria2.rs`, `youtube.rs`, dan `universal.rs`.
pub fn missing_tool_msg(binary: &str, pkg: &str) -> String {
    detect().missing_tool_msg(binary, pkg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── from_ids: Arch & turunannya ──

    #[test]
    fn arch_and_derivatives_map_to_pacman() {
        for id in [
            "arch",
            "manjaro",
            "endeavouros",
            "garuda",
            "cachyos",
            "archarm",
            "ARCH",
            "  Arch  ",
        ] {
            assert_eq!(PackageManager::from_ids(&[id]), PackageManager::Pacman);
        }
    }

    #[test]
    fn id_like_arch_is_enough_for_derivatives() {
        // Turunan Arch sering tidak menyebut "arch" di ID — ID_LIKE yang bawa.
        let pm = PackageManager::from_ids(&["somebrandnewspin", "arch"]);
        assert_eq!(pm, PackageManager::Pacman);
    }

    // ── from_ids: keluarga lain ──

    #[test]
    fn other_families_map_to_their_manager() {
        let cases = [
            ("debian", PackageManager::Apt),
            ("ubuntu", PackageManager::Apt),
            ("linuxmint", PackageManager::Apt),
            ("fedora", PackageManager::Dnf),
            ("rocky", PackageManager::Dnf),
            ("opensuse-leap", PackageManager::Zypper),
            ("opensuse-tumbleweed", PackageManager::Zypper),
            ("void", PackageManager::Xbps),
            ("alpine", PackageManager::Apk),
        ];
        for (id, want) in cases {
            assert_eq!(PackageManager::from_ids(&[id]), want, "ID {id:?}");
        }
    }

    #[test]
    fn unknown_id_does_not_guess() {
        assert_eq!(PackageManager::from_ids(&[]), PackageManager::Unknown);
        // Distro di luar daftar: jangan menebak perintah yang bisa salah.
        let pm = PackageManager::from_ids(&["gentoo", "nixos"]);
        assert_eq!(pm, PackageManager::Unknown);
    }

    // ── parse_os_release ──

    #[test]
    fn parse_os_release_reads_arch() {
        let content = "\
NAME=\"Arch Linux\"
ID=arch
BUILD_ID=rolling
PRETTY_NAME=\"Arch Linux\"
";
        assert_eq!(parse_os_release(content), vec!["arch".to_string()]);
    }

    #[test]
    fn parse_os_release_unquotes_and_splits_id_like() {
        let content = "\
# komentar diabaikan
NAME=\"Linux Mint\"
VERSION=\"21.3 (Virginia)\"
ID=linuxmint
ID_LIKE=\"ubuntu debian\"
";
        let ids = parse_os_release(content);
        assert_eq!(ids, vec!["linuxmint", "ubuntu", "debian"]);
    }

    #[test]
    fn parse_os_release_ignores_other_keys_and_garbage() {
        let content = "PRETTY_NAME=\"Fedora Linux 40\"\nVARIANT_ID=workstation\nbaris-rusak\n";
        assert!(parse_os_release(content).is_empty());
        assert!(parse_os_release("").is_empty());
    }

    #[test]
    fn parse_os_release_dedupes_tokens() {
        let content = "ID=fedora\nID_LIKE=\"fedora rhel\"\n";
        assert_eq!(parse_os_release(content), vec!["fedora", "rhel"]);
    }

    // ── from_binaries (fallback tanpa os-release) ──

    #[test]
    fn binaries_fall_back_in_caller_order() {
        let pacman = PackageManager::from_binaries(&["pacman"]);
        assert_eq!(pacman, PackageManager::Pacman);

        let apt = PackageManager::from_binaries(&["apt-get"]);
        assert_eq!(apt, PackageManager::Apt);

        // Urutan input = prioritas.
        let first = PackageManager::from_binaries(&["dnf", "apt-get"]);
        assert_eq!(first, PackageManager::Dnf);

        let none = PackageManager::from_binaries(&["vim", "nano"]);
        assert_eq!(none, PackageManager::Unknown);
        assert_eq!(PackageManager::from_binaries(&[]), PackageManager::Unknown);
    }

    // ── install_cmd ──

    #[test]
    fn install_cmd_uses_the_right_command_per_manager() {
        use PackageManager::*;
        assert_eq!(Pacman.install_cmd("aria2"), "sudo pacman -S aria2");
        assert_eq!(Apt.install_cmd("yt-dlp"), "sudo apt install yt-dlp");
        assert_eq!(Dnf.install_cmd("aria2"), "sudo dnf install aria2");
        assert_eq!(Zypper.install_cmd("aria2"), "sudo zypper install aria2");
        assert_eq!(Xbps.install_cmd("aria2"), "sudo xbps-install -S aria2");
        assert_eq!(Apk.install_cmd("aria2"), "sudo apk add aria2");
    }

    #[test]
    fn unknown_manager_still_names_the_package() {
        let msg = PackageManager::Unknown.install_cmd("aria2");
        assert!(msg.contains("aria2"), "nama paket harus disebut: {msg}");
        assert!(!msg.contains("apt"), "jangan menyuruh apt: {msg}");
        assert!(!msg.contains("pacman"), "jangan menyuruh pacman: {msg}");
    }

    // ── missing_tool_msg (dipakai aria2.rs / youtube.rs / universal.rs) ──

    #[test]
    fn missing_tool_msg_keeps_binary_and_package_distinct() {
        use PackageManager::*;
        // aria2c adalah binary dari paket "aria2" — jangan tertukar.
        let arch = Pacman.missing_tool_msg("aria2c", "aria2");
        assert_eq!(
            arch,
            "aria2c tidak terinstall — jalankan: sudo pacman -S aria2"
        );

        let debian = Apt.missing_tool_msg("yt-dlp", "yt-dlp");
        assert_eq!(
            debian,
            "yt-dlp tidak terinstall — jalankan: sudo apt install yt-dlp"
        );
    }

    /// Versi publik harus tetap menyebut binary dan paket apa pun hasil
    /// deteksi di mesin yang menjalankan test.
    #[test]
    fn public_missing_tool_msg_is_well_formed() {
        let msg = missing_tool_msg("aria2c", "aria2");
        assert!(msg.starts_with("aria2c tidak terinstall"), "{msg}");
        assert!(msg.contains("aria2"), "nama paket hilang: {msg}");
    }

    /// Deteksi tidak boleh panic walau `/etc/os-release` tidak ada, dan harus
    /// selalu menghasilkan perintah yang tidak kosong.
    #[test]
    fn detect_never_panics() {
        let pm = detect();
        assert!(!pm.install_cmd("aria2").is_empty());
    }
}
