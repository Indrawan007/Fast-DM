//! Petunjuk pacman untuk target Arch Linux.
//!
//! Nama binary (`aria2c`) dan nama paket Arch (`aria2`) tetap dipisahkan agar
//! pesan instalasi akurat.

/// Perintah pacman untuk memasang paket yang diperlukan.
pub fn install_hint(pkg: &str) -> String {
    format!("sudo pacman -S --needed {pkg}")
}

/// Pesan siap-tampil untuk binary downloader yang tidak ditemukan.
/// `binary` = nama executable (`aria2c`), `pkg` = nama paket (`aria2`).
pub fn missing_tool_msg(binary: &str, pkg: &str) -> String {
    format!(
        "{binary} tidak terinstall — jalankan: {}",
        install_hint(pkg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_hint_uses_arch_pacman() {
        assert_eq!(install_hint("aria2"), "sudo pacman -S --needed aria2");
        assert_eq!(install_hint("yt-dlp"), "sudo pacman -S --needed yt-dlp");
    }

    #[test]
    fn missing_tool_message_separates_binary_and_package_names() {
        assert_eq!(
            missing_tool_msg("aria2c", "aria2"),
            "aria2c tidak terinstall — jalankan: sudo pacman -S --needed aria2"
        );
        assert_eq!(
            missing_tool_msg("yt-dlp", "yt-dlp"),
            "yt-dlp tidak terinstall — jalankan: sudo pacman -S --needed yt-dlp"
        );
    }
}
