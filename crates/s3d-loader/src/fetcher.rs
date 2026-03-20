//! アセット取得コアモジュール
//!
//! 旧 TS の `AssetLoader` に相当する機能を Rust/async で実装する:
//! - `manifest` の HTTP フェッチ（内部キャッシュ付き）
//! - 並列ダウンロード（concurrency 制御）
//! - SHA-256 整合性チェック
//! - リトライ（指数バックオフ）
//! - 進捗コールバック
//! - `CancellationToken` によるキャンセル

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use reqwest::Client;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use s3d_types::loader::{LoadError, LoadErrorKind};
use s3d_types::manifest::DeployManifest;

use crate::strategy::StrategyAsset;

// ─────────────────────────────────────────────
// FetchError
// ─────────────────────────────────────────────

/// fetcher モジュールのエラー型
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP error for `{key}` ({url}): {status}")]
    Http {
        key: String,
        url: String,
        status: u16,
    },

    #[error("Network error for `{key}` ({url}): {cause}")]
    Network {
        key: String,
        url: String,
        cause: String,
    },

    #[error("Integrity mismatch for `{key}`: expected {expected}, got {actual}")]
    Integrity {
        key: String,
        expected: String,
        actual: String,
    },

    #[error("Manifest fetch failed ({url}): {cause}")]
    ManifestFetch { url: String, cause: String },

    #[error("Manifest parse failed ({url}): {cause}")]
    ManifestParse { url: String, cause: String },

    #[error("Cancelled")]
    Cancelled,
}

impl From<FetchError> for LoadError {
    fn from(e: FetchError) -> Self {
        match &e {
            FetchError::Http { key, url, .. } => LoadError {
                kind: LoadErrorKind::Network,
                key: key.clone(),
                url: url.clone(),
                cause: Some(e.to_string()),
                status_code: if let FetchError::Http { status, .. } = &e {
                    Some(*status)
                } else {
                    None
                },
            },
            FetchError::Network { key, url, .. } => LoadError {
                kind: LoadErrorKind::Network,
                key: key.clone(),
                url: url.clone(),
                cause: Some(e.to_string()),
                status_code: None,
            },
            FetchError::Integrity { key, .. } => LoadError {
                kind: LoadErrorKind::Integrity,
                key: key.clone(),
                url: String::new(),
                cause: Some(e.to_string()),
                status_code: None,
            },
            FetchError::Cancelled => LoadError {
                kind: LoadErrorKind::Abort,
                key: String::new(),
                url: String::new(),
                cause: Some("Cancelled".to_string()),
                status_code: None,
            },
            _ => LoadError {
                kind: LoadErrorKind::Unknown,
                key: String::new(),
                url: String::new(),
                cause: Some(e.to_string()),
                status_code: None,
            },
        }
    }
}

// ─────────────────────────────────────────────
// CancellationToken
// ─────────────────────────────────────────────

/// キャンセルシグナル（AbortSignal 相当）
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// キャンセルをトリガーする
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// キャンセル済みか判定する
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

// ─────────────────────────────────────────────
// FetchOptions
// ─────────────────────────────────────────────

/// アセット取得のオプション
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// 並列ダウンロード数（デフォルト: 4）
    pub concurrency: usize,
    /// リトライ回数（デフォルト: 3）
    pub retry_count: u32,
    /// リトライ基本遅延（ミリ秒、指数バックオフのベース、デフォルト: 500）
    pub retry_base_delay_ms: u64,
    /// タイムアウト（ミリ秒、デフォルト: 30_000）
    pub timeout_ms: u64,
    /// SHA-256 整合性チェックを有効にするか（デフォルト: true）
    pub integrity_check: bool,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            retry_count: 3,
            retry_base_delay_ms: 500,
            timeout_ms: 30_000,
            integrity_check: true,
        }
    }
}

// ─────────────────────────────────────────────
// ProgressEvent
// ─────────────────────────────────────────────

/// 進捗コールバックに渡されるイベント
#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub key: String,
    pub loaded_count: usize,
    pub total_count: usize,
}

// ─────────────────────────────────────────────
// Fetcher
// ─────────────────────────────────────────────

/// アセット取得コア
///
/// `reqwest::Client` を共有し、manifest キャッシュを内部に保持する。
pub struct Fetcher {
    client: Client,
    opts: FetchOptions,
    /// フェッチ済み manifest のインメモリキャッシュ（URL → DeployManifest）
    manifest_cache: Mutex<std::collections::HashMap<String, DeployManifest>>,
}

impl Fetcher {
    /// 新しい Fetcher を作成する
    pub fn new(opts: FetchOptions) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(opts.timeout_ms))
            .build()
            .expect("failed to build reqwest client");

        Self {
            client,
            opts,
            manifest_cache: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// manifest JSON を取得・パースし、内部キャッシュに保存する
    pub async fn fetch_manifest(&self, url: &str) -> Result<DeployManifest, FetchError> {
        {
            let cache = self.manifest_cache.lock().await;
            if let Some(m) = cache.get(url) {
                return Ok(m.clone());
            }
        }

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::ManifestFetch {
                url: url.to_string(),
                cause: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(FetchError::ManifestFetch {
                url: url.to_string(),
                cause: format!("HTTP {}", resp.status()),
            });
        }

        let manifest: DeployManifest =
            resp.json().await.map_err(|e| FetchError::ManifestParse {
                url: url.to_string(),
                cause: e.to_string(),
            })?;

        self.manifest_cache
            .lock()
            .await
            .insert(url.to_string(), manifest.clone());

        Ok(manifest)
    }

    /// 内部 manifest キャッシュをクリアする（再フェッチ強制）
    pub async fn invalidate_manifest_cache(&self, url: &str) {
        self.manifest_cache.lock().await.remove(url);
    }

    /// 1アセットをリトライ付きでダウンロードし、整合性チェックを行う
    async fn fetch_one(
        &self,
        key: &str,
        url: &str,
        expected_hash: &str,
        token: &CancellationToken,
    ) -> Result<Vec<u8>, FetchError> {
        let mut attempt = 0u32;

        loop {
            if token.is_cancelled() {
                return Err(FetchError::Cancelled);
            }

            match self.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let data = resp.bytes().await.map_err(|e| FetchError::Network {
                        key: key.to_string(),
                        url: url.to_string(),
                        cause: e.to_string(),
                    })?;

                    // SHA-256 整合性チェック
                    if self.opts.integrity_check {
                        let mut hasher = Sha256::new();
                        hasher.update(&data);
                        let actual = hex::encode(hasher.finalize());
                        // expected_hash は任意長プレフィックス（8文字など）
                        if !actual.starts_with(expected_hash) && actual != expected_hash {
                            return Err(FetchError::Integrity {
                                key: key.to_string(),
                                expected: expected_hash.to_string(),
                                actual: actual[..expected_hash.len().min(actual.len())].to_string(),
                            });
                        }
                    }

                    return Ok(data.to_vec());
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    // 4xx はリトライしない
                    if status >= 400 && status < 500 {
                        return Err(FetchError::Http {
                            key: key.to_string(),
                            url: url.to_string(),
                            status,
                        });
                    }
                    // 5xx はリトライ対象
                    if attempt >= self.opts.retry_count {
                        return Err(FetchError::Http {
                            key: key.to_string(),
                            url: url.to_string(),
                            status,
                        });
                    }
                }
                Err(e) => {
                    if attempt >= self.opts.retry_count {
                        return Err(FetchError::Network {
                            key: key.to_string(),
                            url: url.to_string(),
                            cause: e.to_string(),
                        });
                    }
                }
            }

            // 指数バックオフ
            let delay = self.opts.retry_base_delay_ms * (1u64 << attempt.min(6));
            tokio::time::sleep(Duration::from_millis(delay)).await;
            attempt += 1;
        }
    }

    /// 複数アセットを並列ダウンロードする
    ///
    /// - `assets`: `(key, url, expected_hash)` のリスト
    /// - `on_progress`: 各アセット完了時に呼ばれるコールバック（オプション）
    /// - `token`: キャンセルシグナル
    pub async fn fetch_all(
        &self,
        assets: Vec<(String, String, String)>,
        on_progress: Option<Arc<dyn Fn(ProgressEvent) + Send + Sync>>,
        token: CancellationToken,
    ) -> Vec<Result<StrategyAsset, FetchError>> {
        let total = assets.len();
        let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        stream::iter(assets)
            .map(|(key, url, hash)| {
                let token = token.clone();
                let progress = on_progress.clone();
                let completed = completed.clone();
                async move {
                    let result = self.fetch_one(&key, &url, &hash, &token).await;
                    match result {
                        Ok(data) => {
                            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(cb) = progress {
                                cb(ProgressEvent {
                                    key: key.clone(),
                                    loaded_count: n,
                                    total_count: total,
                                });
                            }
                            Ok(StrategyAsset {
                                key,
                                url,
                                hash,
                                size: data.len() as u64,
                                data,
                            })
                        }
                        Err(e) => Err(e),
                    }
                }
            })
            .buffer_unordered(self.opts.concurrency)
            .collect()
            .await
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_token_works() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn fetch_options_defaults() {
        let opts = FetchOptions::default();
        assert_eq!(opts.concurrency, 4);
        assert_eq!(opts.retry_count, 3);
        assert!(opts.integrity_check);
    }

    #[test]
    fn fetch_error_converts_to_load_error() {
        let fe = FetchError::Integrity {
            key: "a.js".to_string(),
            expected: "abc".to_string(),
            actual: "xyz".to_string(),
        };
        let le: LoadError = fe.into();
        assert_eq!(le.kind, LoadErrorKind::Integrity);
    }
}
