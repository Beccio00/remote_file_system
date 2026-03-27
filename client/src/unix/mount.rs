use super::remote_fs::RemoteFS;
use crate::types::CacheConfig;
use fuser::MountOption;

/// Mounts the FUSE filesystem with options tailored for remote access.
pub fn run(mountpoint: &str, server_url: &str, cache: CacheConfig) {
    println!("Mounting at: {}", mountpoint);
    println!("Server: {}", server_url);
    println!(
        "Cache: dir_ttl={}s, file_ttl={}s, max={}MB",
        cache.dir_ttl.as_secs(),
        cache.file_ttl.as_secs(),
        cache.max_file_cache_bytes / 1024 / 1024,
    );

    let fs = RemoteFS::new(server_url, cache);

    // Core mount configuration shared across Unix targets.
    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::Subtype("remote-fs".to_string()),
        MountOption::DefaultPermissions,
        MountOption::AllowOther,
        MountOption::AutoUnmount,
    ];

    #[cfg(target_os = "macos")]
    {
        options.push(MountOption::CUSTOM("noappledouble".to_string()));
        options.push(MountOption::CUSTOM("noapplexattr".to_string()));
        options.push(MountOption::CUSTOM("nobrowse".to_string()));
    }

    if let Err(e) = fuser::mount2(fs, mountpoint, &options) {
        eprintln!("Mount failed: {}", e);
        eprintln!("Ensure the mount point exists and you have the necessary permissions.");
        std::process::exit(1);
    }
}
