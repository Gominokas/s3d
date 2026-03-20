//! AWS Signature Version 4 署名ヘルパー
//!
//! PUT/GET/DELETE/LIST リクエストに必要な Authorization ヘッダーを生成する。

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// 署名設定
pub struct SignConfig {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub service: String,
    pub method: String,
    pub bucket: String,
    pub key: String,
    pub endpoint: String,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// リクエストに付与するヘッダーマップを返す
pub fn sign_request(cfg: &SignConfig) -> Result<BTreeMap<String, String>> {
    let now = Utc::now();
    let date_stamp = now.format("%Y%m%d").to_string(); // 20260101
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string(); // 20260101T120000Z

    // ── ペイロードハッシュ
    let payload_hash = hex_sha256(&cfg.body);

    // ── Canonical URI
    let uri = if cfg.key.is_empty() {
        format!("/{}/", cfg.bucket)
    } else {
        format!("/{}/{}", cfg.bucket, encode_uri(&cfg.key))
    };

    // ── Canonical Query String (list 時は呼び出し元が URL に付与済み)
    let canonical_qs = "".to_string();

    // ── ホスト
    let host = extract_host(&cfg.endpoint);

    // ── 署名対象ヘッダー
    let mut signed_headers_map: BTreeMap<String, String> = BTreeMap::new();
    signed_headers_map.insert("host".to_string(), host.clone());
    signed_headers_map.insert("x-amz-content-sha256".to_string(), payload_hash.clone());
    signed_headers_map.insert("x-amz-date".to_string(), amz_date.clone());
    if let Some(ct) = &cfg.content_type {
        signed_headers_map.insert("content-type".to_string(), ct.clone());
    }

    let signed_headers_str: String = signed_headers_map
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(";");

    let canonical_headers: String = signed_headers_map
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect();

    // ── Canonical Request
    let canonical_request = format!(
        "{method}\n{uri}\n{qs}\n{headers}\n{signed_headers}\n{payload}",
        method = cfg.method,
        uri = uri,
        qs = canonical_qs,
        headers = canonical_headers,
        signed_headers = signed_headers_str,
        payload = payload_hash,
    );

    // ── String To Sign
    let scope = format!(
        "{date_stamp}/{region}/{service}/aws4_request",
        region = cfg.region,
        service = cfg.service
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{cr_hash}",
        cr_hash = hex_sha256(canonical_request.as_bytes()),
    );

    // ── 署名鍵
    let signing_key = derive_signing_key(&cfg.secret_key, &date_stamp, &cfg.region, &cfg.service)?;
    let signature = hmac_sha256_hex(&signing_key, string_to_sign.as_bytes())?;

    // ── Authorization ヘッダー
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={sig}",
        access_key = cfg.access_key,
        scope = scope,
        signed_headers = signed_headers_str,
        sig = signature,
    );

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    headers.insert("Authorization".to_string(), authorization);
    headers.insert("x-amz-date".to_string(), amz_date);
    headers.insert("x-amz-content-sha256".to_string(), payload_hash);
    headers.insert("Host".to_string(), host);
    if let Some(ct) = &cfg.content_type {
        headers.insert("Content-Type".to_string(), ct.clone());
    }

    Ok(headers)
}

// ──────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(data);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn hmac_sha256_raw(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Result<Vec<u8>> {
    let k_date = hmac_sha256_raw(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let k_region = hmac_sha256_raw(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256_raw(&k_region, service.as_bytes())?;
    let k_signing = hmac_sha256_raw(&k_service, b"aws4_request")?;
    Ok(k_signing)
}

fn extract_host(endpoint: &str) -> String {
    endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

/// URI エンコード（スラッシュはエンコードしない）
fn encode_uri(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_sha256_empty() {
        // SHA-256 of empty string
        assert_eq!(
            hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://abc123.r2.cloudflarestorage.com"),
            "abc123.r2.cloudflarestorage.com"
        );
        assert_eq!(extract_host("http://localhost:9000/"), "localhost:9000");
    }

    #[test]
    fn test_sign_request_returns_headers() {
        let cfg = SignConfig {
            access_key: "AKID".to_string(),
            secret_key: "SECRET".to_string(),
            region: "auto".to_string(),
            service: "s3".to_string(),
            method: "PUT".to_string(),
            bucket: "my-bucket".to_string(),
            key: "app.js".to_string(),
            endpoint: "https://example.r2.cloudflarestorage.com".to_string(),
            content_type: Some("application/javascript".to_string()),
            body: b"console.log(1)".to_vec(),
        };
        let headers = sign_request(&cfg).unwrap();
        assert!(headers.contains_key("Authorization"));
        assert!(headers.contains_key("x-amz-date"));
        assert!(headers.contains_key("x-amz-content-sha256"));
    }
}
