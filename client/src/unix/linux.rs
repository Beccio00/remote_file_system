use crate::cli::Cli;
use fuser::MountOption;
use super::remote_fs::RemoteFS;

/// Linux entry point that resolves cache settings and starts mounting.
pub fn run(cli: &Cli) {
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
    ];

    if let Err(e) = fuser::mount2(fs, &cli.mountpoint, &options) {
        eprintln!("Mount failed: {}", e);
        eprintln!("Ensure the mount point exists and you have the necessary permissions.");
        std::process::exit(1);
    }
}
