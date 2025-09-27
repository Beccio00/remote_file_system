use crate::common::RemoteFS;
use fuser::MountOption;

pub fn run(mountpoint: &str) {
    if !std::path::Path::new("/Library/Frameworks/macFUSE.framework").exists() {
        eprintln!("macFUSE is not installed.");
        eprintln!("Please install it with: brew install --cask macfuse");
        eprintln!("Then enable it in System Preferences > Privacy & Security");
        std::process::exit(1);
    }

    println!("Starting Remote File System on macOS...");

    run_linux_macos(mountpoint);
}
