use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
/// Entry metadata returned by the remote server for a directory listing.
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Runtime cache policy used by the client filesystem layer.
pub struct CacheConfig {
    pub dir_ttl: Duration,
    pub file_ttl: Duration,
    pub max_file_cache_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            dir_ttl: Duration::from_secs(5),
            file_ttl: Duration::from_secs(10),
            max_file_cache_bytes: 64 * 1024 * 1024,
        }
    }
}

impl CacheConfig {
    /// Builds cache settings from CLI flags, including no-cache mode.
    pub fn from_cli(no_cache: bool, dir_ttl: u64, file_ttl: u64, max_mb: usize) -> Self {
        if no_cache {
            Self {
                dir_ttl: Duration::from_millis(100),
                file_ttl: Duration::from_millis(100),
                max_file_cache_bytes: 0,
            }
        } else {
            Self {
                dir_ttl: Duration::from_secs(dir_ttl),
                file_ttl: Duration::from_secs(file_ttl),
                max_file_cache_bytes: max_mb * 1024 * 1024,
            }
        }
    }
}

#[allow(dead_code)]
/// Joins a parent path and child name using the remote path format.
pub fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
}

/// Returns the parent directory of a remote path.
pub fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => path[..pos].to_string(),
        None => String::new(),
    }
}
