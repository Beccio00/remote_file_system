use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

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
    pub fn from_cli(no_cache: bool, dir_ttl: u64, file_ttl: u64, max_mb: usize) -> Self {
        if no_cache {
            Self {
                dir_ttl: Duration::ZERO,
                file_ttl: Duration::ZERO,
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
pub fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
}

pub fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => path[..pos].to_string(),
        None => String::new(),
    }
}
