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
    println!("Mounting at: {}", mountpoint);

    let fs = RemoteFS::new("http://127.0.0.1:8000");

    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::Subtype("remote-fs".to_string()),
        MountOption::DefaultPermissions,
        MountOption::AllowOther,
    ];
    if options.contains(&MountOption::AutoUnmount) {
        println!("Auto-unmount on exit is ENABLED ✅");
    } else {
        println!("Auto-unmount is DISABLED ❌ (use --auto-unmount to enable)");
    }

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
