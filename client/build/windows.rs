fn main() {
    // Enables delayed WinFSP loading on Windows targets.
    #[cfg(target_os = "windows")]
    winfsp::build::winfsp_link_delayload();
}
