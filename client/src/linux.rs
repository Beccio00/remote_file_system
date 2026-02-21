use crate::Cli;
use crate::common::{CacheConfig, run_linux_macos};
use std::time::Duration;

pub fn run(cli: &Cli) {
    println!("Starting Remote File System on Linux...");

    let cache_config = if cli.no_cache {
        CacheConfig {
            dir_ttl: Duration::ZERO,
            file_ttl: Duration::ZERO,
            max_file_cache_bytes: 0,
        }
    } else {
        CacheConfig {
            dir_ttl: Duration::from_secs(cli.dir_cache_ttl),
            file_ttl: Duration::from_secs(cli.file_cache_ttl),
            max_file_cache_bytes: cli.max_cache_mb * 1024 * 1024,
        }
    };

    run_linux_macos(&cli.mountpoint, &cli.server_url, cache_config);
}




