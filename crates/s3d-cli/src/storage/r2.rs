//! Cloudflare R2 / AWS S3 互換ストレージ実装
//!
//! `reqwest` と自前の AWS Signature V4 署名を使い、
//! S3 互換エンドポイントへ接続する。
//! 実際の接続が必要なテストは `#[ignore]` でスキップする。

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use s3d_types::plugin::{StorageError, StoragePlugin, StorageResult};

use crate::storage::credentials::StorageCredentials;
use crate::storage::sign::{sign_request, SignConfig};

// ──────────────────────────────────────────────────────────────
// R2Storage
// ──────────────────────────────────────────────────────────────

/// Cloudflare R2 / S3 互換ストレージ
pub struct R2Storage {
    client: Client,
    creds: StorageCredentials,
    bucket: String,
    endpoint: String,
    region: String,
}

impl R2Storage {
    /// 認証情報とエンドポイントから R2 クライアントを構築する
    ///
    /// `endpoint`: `https://<account_id>.r2.cloudflarestorage.com`
    pub fn new(
        creds: StorageCredentials,
        bucket: String,
        endpoint: String,
        region: Option<String>,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self {
            client,
            creds,
            bucket,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            region: region.unwrap_or_else(|| "auto".to_string()),
        })
    }

    fn url(&self, key: &str) -> String {
        format!("{}/{}/{}", self.endpoint, self.bucket, key)
    }

    fn sign_cfg(
        &self,
        method: &str,
        key: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> SignConfig {
        SignConfig {
            access_key: self.creds.access_key_id.clone(),
            secret_key: self.creds.secret_access_key.clone(),
            region: self.region.clone(),
            service: "s3".to_string(),
            method: method.to_string(),
            bucket: self.bucket.clone(),
            key: key.to_string(),
            endpoint: self.endpoint.clone(),
            content_type: content_type.map(|s| s.to_string()),
            body: body.to_vec(),
        }
    }
}

// ──────────────────────────────────────────────────────────────
// StoragePlugin impl
// ──────────────────────────────────────────────────────────────

#[async_trait]
impl StoragePlugin for R2Storage {
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()> {
        let url = self.url(key);
        let sign_cfg = self.sign_cfg("PUT", key, Some(content_type), data);
        let headers = sign_request(&sign_cfg).map_err(|e| StorageError {
            message: format!("署名エラー: {e}"),
            key: Some(key.into()),
        })?;

        let mut req = self.client.put(&url).body(data.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.map_err(|e| StorageError {
            message: format!("PUT {key} failed: {e}"),
            key: Some(key.into()),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError {
                message: format!("PUT {key} HTTP {status}: {body}"),
                key: Some(key.into()),
            });
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> StorageResult<Vec<u8>> {
        let url = self.url(key);
        let sign_cfg = self.sign_cfg("GET", key, None, &[]);
        let headers = sign_request(&sign_cfg).map_err(|e| StorageError {
            message: format!("署名エラー: {e}"),
            key: Some(key.into()),
        })?;

        let mut req = self.client.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.map_err(|e| StorageError {
            message: format!("GET {key} failed: {e}"),
            key: Some(key.into()),
        })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(StorageError {
                message: format!("{key} not found"),
                key: Some(key.into()),
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError {
                message: format!("GET {key} HTTP {status}: {body}"),
                key: Some(key.into()),
            });
        }
        let bytes = resp.bytes().await.map_err(|e| StorageError {
            message: format!("body read error for {key}: {e}"),
            key: Some(key.into()),
        })?;
        Ok(bytes.to_vec())
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        let url = self.url(key);
        let sign_cfg = self.sign_cfg("DELETE", key, None, &[]);
        let headers = sign_request(&sign_cfg).map_err(|e| StorageError {
            message: format!("署名エラー: {e}"),
            key: Some(key.into()),
        })?;

        let mut req = self.client.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.map_err(|e| StorageError {
            message: format!("DELETE {key} failed: {e}"),
            key: Some(key.into()),
        })?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NO_CONTENT {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError {
                message: format!("DELETE {key} HTTP {status}: {body}"),
                key: Some(key.into()),
            });
        }
        Ok(())
    }

    async fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        // list-objects-v2 クエリ
        let query = format!("list-type=2&prefix={}", urlencoding_simple(prefix));
        let url = format!("{}/?{}", self.url("").trim_end_matches('/'), query);

        let sign_cfg = SignConfig {
            access_key: self.creds.access_key_id.clone(),
            secret_key: self.creds.secret_access_key.clone(),
            region: self.region.clone(),
            service: "s3".to_string(),
            method: "GET".to_string(),
            bucket: self.bucket.clone(),
            key: "".to_string(),
            endpoint: self.endpoint.clone(),
            content_type: None,
            body: vec![],
        };
        let headers = sign_request(&sign_cfg).map_err(|e| StorageError {
            message: format!("署名エラー: {e}"),
            key: None,
        })?;

        let mut req = self.client.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.map_err(|e| StorageError {
            message: format!("LIST failed: {e}"),
            key: None,
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError {
                message: format!("LIST HTTP {status}: {body}"),
                key: None,
            });
        }

        let text = resp.text().await.map_err(|e| StorageError {
            message: format!("body read error: {e}"),
            key: None,
        })?;

        // 簡易 XML パース: <Key>...</Key> を抽出
        let keys = extract_keys_from_xml(&text);
        Ok(keys)
    }
}

// ──────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────

fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

fn extract_keys_from_xml(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        rest = &rest[start + 5..];
        if let Some(end) = rest.find("</Key>") {
            keys.push(rest[..end].to_string());
            rest = &rest[end + 6..];
        } else {
            break;
        }
    }
    keys
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keys_from_xml() {
        let xml = r#"<?xml version="1.0"?>
<ListBucketResult>
  <Contents><Key>app.js</Key></Contents>
  <Contents><Key>style.css</Key></Contents>
</ListBucketResult>"#;
        let keys = extract_keys_from_xml(xml);
        assert_eq!(keys, vec!["app.js", "style.css"]);
    }

    #[test]
    fn test_urlencoding_simple() {
        assert_eq!(urlencoding_simple("foo/bar.js"), "foo/bar.js");
        assert_eq!(urlencoding_simple("foo bar"), "foo%20bar");
    }

    /// 実際の R2 エンドポイントが必要なテストは `#[ignore]`
    #[tokio::test]
    #[ignore]
    async fn test_r2_put_get_delete() {
        let creds = StorageCredentials::from_env().expect("creds");
        let bucket = std::env::var("S3D_BUCKET").unwrap_or_else(|_| "test-bucket".to_string());
        let endpoint = std::env::var("S3D_ENDPOINT")
            .unwrap_or_else(|_| "https://example.r2.cloudflarestorage.com".to_string());
        let storage = R2Storage::new(creds, bucket, endpoint, None).unwrap();

        let key = "s3d-cli-test/hello.txt";
        storage.put(key, b"hello", "text/plain").await.unwrap();
        let data = storage.get(key).await.unwrap();
        assert_eq!(data, b"hello");
        storage.delete(key).await.unwrap();
    }
}
