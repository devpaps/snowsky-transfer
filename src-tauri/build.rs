fn main() {
    tauri_build::build();

    // Link against system libmtp (requires libmtp-devel on Fedora)
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=mtp");
}
