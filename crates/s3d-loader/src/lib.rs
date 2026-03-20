//! s3d-loader — CDN アセット配信戦略ローダー
//!
//! ## 概要
//! `s3d-loader` はアセット配信戦略（`assetsStrategy`）に従い、
//! 初期表示アセットの取得とバックグラウンド CDN 差分取得を行うクレートです。
//!
//! ## `strategy_assets()` フロー
//!
//! 1. **キャッシュ確認** — `initial.sources` がキャッシュにあればそのまま返す
//! 2. **初期取得** — キャッシュミスの場合は CDN から取得し `cache=true` ならキャッシュ保存
//! 3. **CDN 差分取得** — バックグラウンドで manifest をフェッチし差分アセットのみ取得
//! 4. **リロード** — `reload.trigger` に従い manifest を再フェッチ → 変更があれば差分取得
//!
//! ## 制約
//! - Service Worker 不使用
//! - フレームワーク非依存（React/Vue 等への依存なし）
//! - ブラウザ用 Wasm / CLI 用 native の両方でコンパイル可能

pub mod cache;
pub mod diff;
pub mod fetcher;
pub mod strategy;

pub use cache::{AssetCache, CacheEntry};
pub use diff::{diff_manifests, needs_evict, needs_fetch, DiffEntry};
pub use fetcher::{CancellationToken, FetchError, FetchOptions, Fetcher, ProgressEvent};
pub use strategy::{
    AssetsStrategyConfig, CdnStrategyConfig, InitialConfig, ReloadConfig, ReloadStrategy,
    ReloadTrigger, StrategyAsset,
};

use std::sync::Arc;

use tokio::sync::Mutex;

use s3d_types::manifest::DeployManifest;

// ─────────────────────────────────────────────
// AssetLoader — strategyAssets のエントリポイント
// ─────────────────────────────────────────────

/// `assetsStrategy` / `strategyAssets` のメインローダー
///
/// `Fetcher` と `AssetCache` を内包し、配信戦略に従いアセットを取得する。
pub struct AssetLoader {
    fetcher: Arc<Fetcher>,
    cache: Arc<Mutex<AssetCache>>,
    /// 最後にフェッチした manifest（リロード差分比較用）
    last_manifest: Arc<Mutex<Option<DeployManifest>>>,
}

impl AssetLoader {
    /// 新しい `AssetLoader` を作成する
    pub fn new(opts: FetchOptions) -> Self {
        Self {
            fetcher: Arc::new(Fetcher::new(opts)),
            cache: Arc::new(Mutex::new(AssetCache::new())),
            last_manifest: Arc::new(Mutex::new(None)),
        }
    }

    /// `assetsStrategy` フローに従い初期アセットを取得する
    ///
    /// ## フロー
    /// 1. `manifest_url` から manifest をフェッチ
    /// 2. `config.initial.sources` をキャッシュから確認（ヒットならスキップ）
    /// 3. キャッシュミスのアセットを CDN から並列取得
    /// 4. `config.initial.cache = true` ならキャッシュに保存
    /// 5. 取得済みアセット一覧を返す
    pub async fn strategy_assets(
        &self,
        manifest_url: &str,
        config: &AssetsStrategyConfig,
        on_progress: Option<Arc<dyn Fn(ProgressEvent) + Send + Sync>>,
        token: CancellationToken,
    ) -> Result<Vec<StrategyAsset>, FetchError> {
        // 1. manifest 取得
        let manifest = self.fetcher.fetch_manifest(manifest_url).await?;

        // 2. initial.sources をキャッシュ確認 + 差分取得リスト構築
        let mut to_fetch: Vec<(String, String, String)> = Vec::new();
        let mut cached_assets: Vec<StrategyAsset> = Vec::new();

        {
            let cache = self.cache.lock().await;
            for source_key in &config.initial.sources {
                if let Some(entry) = manifest.assets.get(source_key) {
                    if let Some(data) = cache.get(source_key, &entry.hash) {
                        // キャッシュヒット
                        cached_assets.push(StrategyAsset {
                            key: source_key.clone(),
                            url: entry.url.clone(),
                            hash: entry.hash.clone(),
                            size: entry.size,
                            data: data.to_vec(),
                        });
                    } else {
                        // キャッシュミス → 取得対象に追加
                        to_fetch.push((source_key.clone(), entry.url.clone(), entry.hash.clone()));
                    }
                }
            }
        }

        // 3. キャッシュミス分を並列取得
        let fetched_results = if to_fetch.is_empty() {
            vec![]
        } else {
            self.fetcher.fetch_all(to_fetch, on_progress, token).await
        };

        // エラーを収集（最初のエラーで早期リターン）
        let mut fetched_assets: Vec<StrategyAsset> = Vec::new();
        for result in fetched_results {
            match result {
                Ok(asset) => fetched_assets.push(asset),
                Err(e) => return Err(e),
            }
        }

        // 4. cache=true なら新規取得アセットをキャッシュ保存
        if config.initial.cache {
            let mut cache = self.cache.lock().await;
            for asset in &fetched_assets {
                cache.put(asset.key.clone(), asset.hash.clone(), asset.data.clone());
            }
        }

        // 5. manifest をキャッシュして差分比較用に保存
        *self.last_manifest.lock().await = Some(manifest);

        // キャッシュヒット分 + 新規取得分を合わせて返す
        let mut all = cached_assets;
        all.extend(fetched_assets);
        Ok(all)
    }

    /// バックグラウンド CDN 差分取得
    ///
    /// 新 manifest をフェッチし、変更のあったアセットだけを取得して
    /// キャッシュを更新する。取得した差分アセット一覧を返す。
    pub async fn fetch_cdn_diff(
        &self,
        manifest_url: &str,
        on_progress: Option<Arc<dyn Fn(ProgressEvent) + Send + Sync>>,
        token: CancellationToken,
    ) -> Result<Vec<StrategyAsset>, FetchError> {
        // 最新 manifest を強制再フェッチ
        self.fetcher.invalidate_manifest_cache(manifest_url).await;
        let new_manifest = self.fetcher.fetch_manifest(manifest_url).await?;

        let old_manifest = self.last_manifest.lock().await.clone();

        // diff 計算
        let entries = diff_manifests(old_manifest.as_ref(), &new_manifest);
        let to_fetch: Vec<(String, String, String)> = needs_fetch(&entries)
            .into_iter()
            .filter_map(|e| Some((e.key.clone(), e.url.clone()?, e.hash.clone()?)))
            .collect();

        // 古いキャッシュを削除
        {
            let mut cache = self.cache.lock().await;
            for e in needs_evict(&entries) {
                cache.evict(&e.key);
            }
        }

        if to_fetch.is_empty() {
            *self.last_manifest.lock().await = Some(new_manifest);
            return Ok(vec![]);
        }

        let results = self.fetcher.fetch_all(to_fetch, on_progress, token).await;

        let mut diff_assets: Vec<StrategyAsset> = Vec::new();
        for result in results {
            match result {
                Ok(asset) => {
                    // キャッシュ更新
                    self.cache.lock().await.put(
                        asset.key.clone(),
                        asset.hash.clone(),
                        asset.data.clone(),
                    );
                    diff_assets.push(asset);
                }
                Err(e) => return Err(e),
            }
        }

        *self.last_manifest.lock().await = Some(new_manifest);
        Ok(diff_assets)
    }

    /// 現在のキャッシュ内のアセット数を返す
    pub async fn cached_count(&self) -> usize {
        self.cache.lock().await.len()
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_loader_creates_successfully() {
        let _loader = AssetLoader::new(FetchOptions::default());
    }

    #[test]
    fn cancellation_token_clone_shares_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn cache_count_starts_at_zero() {
        let loader = AssetLoader::new(FetchOptions::default());
        assert_eq!(loader.cached_count().await, 0);
    }
}
