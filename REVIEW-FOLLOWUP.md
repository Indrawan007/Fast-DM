# Tindak lanjut review v2.10.4

Baseline review: 24f8246. Patch kerja: v2.10.5.

## Diimplementasikan

- H1: tail stderr UTF-8-safe bersama, batas satu baris subprocess.
- H2: decode ACK native host wajib success boolean, batas respons; read failure tidak otomatis mengirim ulang aksi.
- H3: cookie jar beratribut end-to-end, matching Secure/path/expiry/host-only/HttpOnly; cookie partitioned dilewati. Payload lama tanpa atribut ditolak. **Atribut file cookie lama yang sudah hilang tidak dapat direkonstruksi; ekspor ulang melalui extension terbaru diperlukan.**
- H4: clear worker aktif mempertahankan tombstone/count sampai supervisor selesai; UI tidak menampilkan tombstone.
- M1: update konfigurasi ke daemon owned, opsi per-URI eksplisit, promosi mengisi kapasitas, port live ditolak. Rollback daemon best-effort jika apply/save gagal; rekonsiliasi daemon asing/reused dan fault injection belum selesai.
- M2: satu poll statistik berurutan, ID terhapus ditolak oleh event listener.
- M3: temporary unik mode 0600 sejak create, write/sync/rename untuk config/session/cookie. Durability direktori saat power loss belum dijamin.
- M4: discovery profil Rust menggunakan XDG_CONFIG_HOME. Pemetaan profil khusus Thorium masih perlu perbaikan.
- M5: provenance nama eksplisit dan resolver tidak menimpanya.
- M6: semua pekerjaan non-terminal dipertahankan, cap hanya Completed/Cancelled.
- Tambahan: URL control character ditolak; add saat shutdown tidak diakui sukses; cap koneksi IPC dan timeout baca; CI build locked dan validasi tag-versi rilis.

## Belum selesai — jangan dianggap sudah diperbaiki

1. Recovery RPC ketika response hilang/tellStatus gagal/unpause gagal/forcePause gagal: rekonsiliasi status sebelum membuat task baru dan jaminan cleanup saat remove gagal.
2. Join supervisor pada shutdown; penghentian/reap process group pada seluruh jalur EOF/timeout, serta tes child mock yang menutup stdout tetapi mempertahankan stderr.
3. Migrasi aman cache cookie lama; cookie partitioned/top-level context; test browser sungguhan dan regresi login lintas situs.
4. Sinkronisasi global daemon reused/asing dan fault-injection apply config; rollback konfigurasi masih best-effort.
5. UI E2E: Save As, hapus row saat event beruntun, restart/auto-resume, dan operasi settings selama daemon lambat.
6. Semantik toggle extension enabled pada overlay/manual download, stale kandidat sniffer SPA, serta pemilihan media ketika satu halaman memiliki beberapa player.
7. Timeout xdg-settings dan probe yt-dlp --version; cap stdout dan cleanup process group fetch_formats.
8. Desktop Entry escaping, shell quoting wrapper dev, registry NMH atomic/concurrent updates, dan pemetaan profil Thorium eksplisit.
9. Verifikasi dukungan SOCKS aria2 CLI/RPC terhadap versi tool yang didukung.
10. Release validation dan smoke GUI/browser menyeluruh; Clippy masih advisory. Komentar test historis yang keliru belum seluruhnya dibersihkan.

## Verifikasi

- Lokal: 9 test extension lulus; syntax JavaScript/shell dan git diff --check lulus.
- Rust: formatter WASM digunakan karena Cargo tidak tersedia lokal. Hasil kompilasi/test harus diperiksa melalui CI branch sesi; jangan menyamakan parsing/formatting dengan build.
- Perubahan belum boleh disebut penyelesaian seluruh review sebelum daftar di atas ditutup dengan test reproduksi.
