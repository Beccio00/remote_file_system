mod remote_fs;
mod mount;

use crate::types::CacheConfig;
use crate::Cli;

pub fn run(cli: &Cli) {
    let cache = CacheConfig::from_cli(
        cli.no_cache,
        cli.dir_cache_ttl,
        cli.file_cache_ttl,
        cli.max_cache_mb,
    );
    mount::run(&cli.mountpoint, &cli.server_url, cache);
}

pub fn request_unmount(mountpoint: &str) {
    match mount::request_unmount(mountpoint) {
        Ok(true) => println!("Unmount requested for {}", mountpoint),
        Ok(false) => {
            eprintln!("No active daemon mount found for {}", mountpoint);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to request unmount for {}: {}", mountpoint, e);
            std::process::exit(1);
        }
    }
}
