// src/http_client.rs
use serde::{Deserialize, Serialize};
use reqwest::{Client, Response};
use anyhow::Result;
use bytes::Bytes;

#[derive(Debug, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct HttpClient {
    base_url: String,
    client: Client,
}

impl HttpClient {
    pub fn new(base_url: String, auth_token: Option<String>) -> Self {
        let client = Client::new();
        Self { base_url, client}
    }
    
    fn url(&self, path: &str, endpoint: &str) -> String {
        format!("{}/{}/{}", self.base_url.trim_end_matches('/'), endpoint, path)
    }

    //if we want to handle the authentication

    // fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    //     if let Some(token) = &self.auth_token {
    //         req.bearer_auth(token)
    //     } else {
    //         req
    //     }
    // }

    /// GET /list/<path>
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RemoteEntry>> {
        let url = self.url(path, "list");
        let resp = self.client.get(&url).send().await?;
        let entries = resp.json::<Vec<RemoteEntry>>().await?;
        Ok(entries)
    }

    /// GET /files/<path> (full download)
    pub async fn read_file(&self, path: &str) -> Result<Bytes> {
        let url = self.url(path, "files");
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("read_file failed: {}", resp.status());
        }
        Ok(resp.bytes().await?)
    }

    /// GET /files/<path> (range)
    pub async fn read_range(&self, path: &str, start: u64, end: u64) -> Result<Bytes> {
        let url = self.url(path, "files");
        let resp = self.client.get(&url)
                .header("Range", format!("bytes={}-{}", start, end))
                .send().await?;
        Ok(resp.bytes().await?)
    }

    /// PUT /files/<path>
    pub async fn write_file(&self, path: &str, data: Bytes) -> Result<()> {
        let url = self.url(path, "files");
        let resp = self.client.put(&url).body(data).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("write_file failed: {}", resp.status());
        }
        Ok(())
    }

    /// POST /mkdir/<path>
    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let url = self.url(path, "mkdir");
        let resp = self.client.post(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("mkdir failed: {}", resp.status());
        }
        Ok(())
    }

    /// DELETE /files/<path>
    pub async fn delete(&self, path: &str) -> Result<()> {
        let url = self.url(path, "files");
        let resp = self.client.delete(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("delete failed: {}", resp.status());
        }
        Ok(())
    }
}
