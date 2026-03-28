use crate::cli::Cli;
use fuser::MountOption;
use super::remote_fs::RemoteFS;

/// macOS entry point that validates macFUSE and mounts the filesystem.
#[allow(dead_code)]
pub fn run(cli: &Cli) {
    if !std::path::Path::new("/Library/Frameworks/macFUSE.framework").exists() {
        eprintln!("macFUSE is not installed.");
        eprintln!("Install with: brew install --cask macfuse");
        std::process::exit(1);
    }

    let cache = cli.cache_config();

    println!("Mounting at: {}", cli.mountpoint);
    println!("Server: {}", cli.server_url);
    println!(
        "Cache: dir_ttl={}s, file_ttl={}s, max={}MB",
        cache.dir_ttl.as_secs(),
        cache.file_ttl.as_secs(),
        cache.max_file_cache_bytes / 1024 / 1024,
    );

    let fs = RemoteFS::new(&cli.server_url, cache);
    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::Subtype("remote-fs".to_string()),
        MountOption::DefaultPermissions,
        MountOption::AllowOther,
        MountOption::AutoUnmount,
        MountOption::CUSTOM("noappledouble".to_string()),
        MountOption::CUSTOM("noapplexattr".to_string()),
        MountOption::CUSTOM("nobrowse".to_string()),
    ];

    if let Err(e) = fuser::mount2(fs, &cli.mountpoint, &options) {
        eprintln!("Mount failed: {}", e);
        eprintln!("Ensure the mount point exists and you have the necessary permissions.");
        std::process::exit(1);
    }
}
