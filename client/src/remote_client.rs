use crate::types::{CacheConfig, RemoteEntry, parent_of};
use reqwest::blocking::Client;
use std::collections::HashMap;
use std::io::Read;
use std::time::Instant;

struct CachedDir {
    entries: Vec<RemoteEntry>,
    cached_at: Instant,
}

struct CachedFile {
    data: Vec<u8>,
    cached_at: Instant,
}

#[allow(dead_code)]
pub struct ProgressReader<R: Read> {
    pub inner: R,
    pub total: u64,
    pub sent: u64,
    pub name: String,
    pub last_pct: u64,
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.sent += n as u64;
        let pct = if self.total > 0 { self.sent * 100 / self.total } else { 100 };
        if pct != self.last_pct {
            self.last_pct = pct;
            let filled = (pct as usize * 30) / 100;
            eprint!("\r\x1b[K  {} [{}>{} ] {}% ({}/{}MB)",
                self.name,
                "=".repeat(filled),
                " ".repeat(30 - filled),
                pct,
                self.sent / (1024 * 1024),
                self.total / (1024 * 1024),
            );
        }
        if n == 0 && self.sent >= self.total {
            eprintln!(" done");
        }
        Ok(n)
    }
}

/// Cross-platform HTTP client that communicates with the remote file server.
/// Handles caching (directory listings + file contents).
pub struct RemoteClient {
    client: Client,
    base_url: String,
    pub cache_config: CacheConfig,
    dir_cache: HashMap<String, CachedDir>,
    file_cache: HashMap<String, CachedFile>,
    file_cache_size: usize,
}

impl RemoteClient {
    pub fn new(base_url: &str, cache_config: CacheConfig) -> Self {
        Self {
            client: Client::builder()
                .timeout(None)
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.to_string(),
            cache_config,
            dir_cache: HashMap::new(),
            file_cache: HashMap::new(),
            file_cache_size: 0,
        }
    }

    #[allow(dead_code)]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    #[allow(dead_code)]
    pub fn http_client(&self) -> &Client {
        &self.client
    }

    pub fn list_dir(&mut self, path: &str) -> Result<Vec<RemoteEntry>, anyhow::Error> {
        if let Some(cached) = self.dir_cache.get(path) {
            if cached.cached_at.elapsed() < self.cache_config.dir_ttl {
                return Ok(cached.entries.clone());
            }
        }

        let url = format!("{}/list/{}", self.base_url, path);
        let entries: Vec<RemoteEntry> = self.client.get(&url).send()?.error_for_status()?.json()?;

        self.dir_cache.insert(path.to_string(), CachedDir {
            entries: entries.clone(),
            cached_at: Instant::now(),
        });
        Ok(entries)
    }

    pub fn fetch_file(&mut self, path: &str) -> Result<Vec<u8>, anyhow::Error> {
        if let Some(cached) = self.file_cache.get(path) {
            if cached.cached_at.elapsed() < self.cache_config.file_ttl {
                return Ok(cached.data.clone());
            }
        }

        let url = format!("{}/files/{}", self.base_url, path);
        let data = self.client.get(&url).send()?.error_for_status()?.bytes()?.to_vec();

        // Evict oldest entries if over budget
        while self.file_cache_size + data.len() > self.cache_config.max_file_cache_bytes {
            let oldest = self.file_cache.iter()
                .min_by_key(|(_, v)| v.cached_at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => {
                    if let Some(evicted) = self.file_cache.remove(&key) {
                        self.file_cache_size -= evicted.data.len();
                    }
                }
                None => break,
            }
        }

        self.file_cache_size += data.len();
        self.file_cache.insert(path.to_string(), CachedFile {
            data: data.clone(),
            cached_at: Instant::now(),
        });
        Ok(data)
    }

    pub fn fetch_range(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        let end = offset + (size as u64) - 1;
        let range_header = format!("bytes={}-{}", offset, end);
        let resp = self.client.get(&url)
            .header("Range", range_header)
            .send()?
            .error_for_status()?;
        Ok(resp.bytes()?.to_vec())
    }

    pub fn upload(&self, path: &str, data: Vec<u8>) -> Result<(), anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        self.client.put(&url).body(data).send()?.error_for_status()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn upload_streamed(&self, path: &str, reader: impl Read + Send + 'static, size: u64) -> Result<(), anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        let body = reqwest::blocking::Body::sized(reader, size);
        self.client.put(&url).body(body).send()?.error_for_status()?;
        Ok(())
    }

    pub fn delete_remote(&self, path: &str) -> Result<(), anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        self.client.delete(&url).send()?.error_for_status()?;
        Ok(())
    }

    pub fn mkdir_remote(&self, path: &str) -> Result<(), anyhow::Error> {
        let url = format!("{}/mkdir/{}", self.base_url, path);
        self.client.post(&url).send()?.error_for_status()?;
        Ok(())
    }

    pub fn invalidate(&mut self, path: &str) {
        self.dir_cache.remove(&parent_of(path));
        self.dir_cache.remove(path);
        if let Some(evicted) = self.file_cache.remove(path) {
            self.file_cache_size -= evicted.data.len();
        }
    }

    /// Check if a file is in the cache and still valid, return cached data slice.
    pub fn cached_file_data(&self, path: &str) -> Option<&[u8]> {
        if let Some(cached) = self.file_cache.get(path) {
            if cached.cached_at.elapsed() < self.cache_config.file_ttl {
                return Some(&cached.data);
            }
        }
        None
    }
}
