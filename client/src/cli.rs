use clap::Parser;
use crate::types::CacheConfig;

/// Remote File System — mount a remote filesystem via FUSE
#[derive(Parser, Debug)]
#[command(name = "remote-fs", version, about, long_about = None)]
pub struct Cli {
    /// Local path where the filesystem will be mounted (e.g. /tmp/mnt)
    pub mountpoint: String,

    /// URL of the remote server
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    pub server_url: String,

    /// Directory cache TTL in seconds
    #[arg(long, default_value = "5")]
    pub dir_cache_ttl: u64,

    /// File cache TTL in seconds
    #[arg(long, default_value = "10")]
    pub file_cache_ttl: u64,

    /// Maximum file cache size in MB
    #[arg(long, default_value = "64")]
    pub max_cache_mb: usize,

    /// Disable caching entirely
    #[arg(long, default_value = "false")]
    pub no_cache: bool,

    /// Run as a background daemon
    #[arg(long, default_value = "false")]
    pub daemon: bool,

    #[cfg(windows)]
    /// Request clean unmount of an existing daemon mount at <MOUNTPOINT> (e.g. R:)
    #[arg(long, default_value = "false")]
    pub unmount: bool,
}

impl Cli {
    /// Returns cache settings derived from CLI flags.
    pub fn cache_config(&self) -> CacheConfig {
        CacheConfig::from_cli(
            self.no_cache,
            self.dir_cache_ttl,
            self.file_cache_ttl,
            self.max_cache_mb,
        )
    }
}
