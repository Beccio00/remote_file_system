use crate::common::RemoteFS;
use fuser::MountOption;

pub fn run(mountpoint: &str) {
    // Check if macFUSE is available
    if !std::path::Path::new("/Library/Frameworks/macFUSE.framework").exists() {
        eprintln!("macFUSE is not installed.");
        eprintln!("Please install it with: brew install --cask macfuse");
        eprintln!("Then enable it in System Preferences > Privacy & Security");
        std::process::exit(1);
    }

    println!("Starting Remote File System on macOS...");
    println!("Mounting at: {}", mountpoint);

    let fs = RemoteFS::new("http://127.0.0.1:8000");

    // macOS-specific mount options
    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::Subtype("remote-fs".to_string()),
        // Enable extended attributes on macOS
        MountOption::DefaultPermissions,
        // Allow other users to access the mount (optional)
        MountOption::AllowOther,
    ];

    match fuser::mount2(fs, mountpoint, &options) {
        Ok(()) => {
            println!("File system mounted successfully at {}", mountpoint);
        }
        Err(e) => {
            eprintln!("Failed to mount file system: {}", e);
            eprintln!("Make sure:");
            eprintln!("1. macFUSE is properly installed and enabled");
            eprintln!("2. The mount point exists and is empty");
            eprintln!("3. You have the necessary permissions");
            std::process::exit(1);
        }
    }
}
