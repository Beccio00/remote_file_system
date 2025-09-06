mod fs;

fn main() {
    
    #[cfg(target_os = "linux")]
    let adapter = fs::LinuxFuseAdapter;

    #[cfg(target_os = "macos")]
    let adapter = fs::MacOSFuseAdapter;

    #[cfg(target_os = "windows")]
    let adapter = fs::WindowsFuseAdapter;

}