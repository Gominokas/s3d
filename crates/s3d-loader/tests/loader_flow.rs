//! s3d-loader 統合テスト
//!
//! mockito を使ったモック HTTP サーバーで以下のフローをテストする:
//!
//! 1. `strategy_assets()` — initial 取得 → キャッシュ保存
//! 2. キャッシュヒット — 2回目の `strategy_assets()` でキャッシュを使用
//! 3. `fetch_cdn_diff()` — 新 manifest で変更のあったアセットだけを取得
//! 4. 進捗コールバック — 3ファイル分のコールバックが呼ばれること

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use s3d_loader::{
    AssetLoader, AssetsStrategyConfig, CancellationToken, CdnStrategyConfig, FetchOptions,
    InitialConfig, ProgressEvent, ReloadConfig, ReloadStrategy, ReloadTrigger,
};
use s3d_types::manifest::{AssetEntry, DeployManifest};

// ─────────────────────────────────────────────
// ヘルパー
// ─────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// アセットデータの一覧からマニフェストを生成する
/// キー: 元のキー（例: "js/main.js"）
/// URL: base_url + "/" + hashed_key（例: "js/main.<hash8>.js"）
fn make_manifest(base_url: &str, entries: &[(&str, &[u8])]) -> DeployManifest {
    let mut assets = HashMap::new();
    for (key, data) in entries {
        let hash_full = sha256_hex(data);
        let hash8 = &hash_full[..8];
        let hashed_key = {
            let dot = key.rfind('.').unwrap_or(key.len());
            let (stem, ext) = key.split_at(dot);
            format!("{}.{}{}", stem, hash8, ext)
        };
        assets.insert(
            key.to_string(),
            AssetEntry {
                url: format!("{}/{}", base_url, hashed_key),
                size: data.len() as u64,
                hash: hash8.to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
    }
    DeployManifest {
        schema_version: 1,
        version: "1.0.0".to_string(),
        build_time: "2026-03-20T00:00:00Z".to_string(),
        assets,
        strategies: std::collections::HashMap::new(),
    }
}

fn default_config(sources: Vec<&str>) -> AssetsStrategyConfig {
    AssetsStrategyConfig {
        initial: InitialConfig {
            sources: sources.into_iter().map(String::from).collect(),
            cache: true,
            fallback: None,
        },
        cdn: CdnStrategyConfig {
            files: vec!["**/*.js".to_string()],
            cache: true,
            max_age: None,
        },
        reload: ReloadConfig {
            trigger: ReloadTrigger::ManifestChange,
            strategy: ReloadStrategy::Diff,
            interval_ms: None,
        },
    }
}

// ─────────────────────────────────────────────
// Test 1: initial 取得 → キャッシュ保存
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_strategy_assets_initial_fetch() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    let data = b"console.log('hello')";
    let files = [("js/main.js", data.as_ref())];
    let manifest = make_manifest(&base, &files);

    // hash8 を取得して hashed_key を再現
    let hash_full = sha256_hex(data);
    let hash8 = &hash_full[..8];
    let hashed_key = format!("js/main.{}.js", hash8);

    let manifest_json = serde_json::to_string(&manifest).unwrap();

    let _m1 = server
        .mock("GET", "/manifest.json")
        .with_status(200)
        .with_body(manifest_json)
        .create_async()
        .await;

    let _m2 = server
        .mock("GET", &format!("/{}", hashed_key) as &str)
        .with_status(200)
        .with_body(data)
        .create_async()
        .await;

    let loader = AssetLoader::new(FetchOptions {
        integrity_check: true,
        ..Default::default()
    });
    let config = default_config(vec!["js/main.js"]);
    let manifest_url = format!("{}/manifest.json", base);

    let assets = loader
        .strategy_assets(&manifest_url, &config, None, CancellationToken::new())
        .await
        .expect("strategy_assets should succeed");

    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].key, "js/main.js");
    assert_eq!(assets[0].data, data.as_ref());
    assert_eq!(loader.cached_count().await, 1);
}

// ─────────────────────────────────────────────
// Test 2: キャッシュヒット（2回目はアセット HTTP 呼び出しなし）
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_strategy_assets_cache_hit() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    let data = b"cached data";
    let files = [("js/app.js", data.as_ref())];
    let manifest = make_manifest(&base, &files);

    let hash_full = sha256_hex(data);
    let hash8 = &hash_full[..8];
    let hashed_key = format!("js/app.{}.js", hash8);
    let manifest_json = serde_json::to_string(&manifest).unwrap();

    // manifest は2回以上呼ばれる（内部キャッシュが manifest 側にあるので1回のみだが許容）
    let _m_manifest = server
        .mock("GET", "/manifest.json")
        .with_status(200)
        .with_body(manifest_json)
        .expect_at_least(1)
        .create_async()
        .await;

    // アセットは1回だけ（2回目はキャッシュヒット）
    let _m_asset = server
        .mock("GET", &format!("/{}", hashed_key) as &str)
        .with_status(200)
        .with_body(data)
        .expect(1)
        .create_async()
        .await;

    let loader = AssetLoader::new(FetchOptions {
        integrity_check: true,
        ..Default::default()
    });
    let config = default_config(vec!["js/app.js"]);
    let manifest_url = format!("{}/manifest.json", base);

    // 1回目: ネットワーク取得
    let assets1 = loader
        .strategy_assets(&manifest_url, &config, None, CancellationToken::new())
        .await
        .expect("first call should succeed");
    assert_eq!(assets1[0].data, data.as_ref());

    // 2回目: キャッシュヒット
    let assets2 = loader
        .strategy_assets(&manifest_url, &config, None, CancellationToken::new())
        .await
        .expect("second call should succeed");
    assert_eq!(assets2[0].data, data.as_ref());

    // モックの expect(1) が満たされていることを assert
    _m_asset.assert_async().await;
}

// ─────────────────────────────────────────────
// Test 3: fetch_cdn_diff — 変更ファイルのみ取得
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_fetch_cdn_diff_only_modified() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    let data_v1: &[u8] = b"version 1";
    let data_v2: &[u8] = b"version 2 updated";
    let data_static: &[u8] = b"static file unchanged";

    // 旧 manifest（v1）
    let old_manifest = make_manifest(
        &base,
        &[("js/main.js", data_v1), ("js/static.js", data_static)],
    );
    // 新 manifest（main.js が v2 に変更）
    let new_manifest = make_manifest(
        &base,
        &[("js/main.js", data_v2), ("js/static.js", data_static)],
    );

    let old_json = serde_json::to_string(&old_manifest).unwrap();
    let new_json = serde_json::to_string(&new_manifest).unwrap();

    // manifest: 1回目は旧、2回目以降は新
    let _m_old = server
        .mock("GET", "/manifest.json")
        .with_status(200)
        .with_body(old_json)
        .expect(1)
        .create_async()
        .await;
    let _m_new = server
        .mock("GET", "/manifest.json")
        .with_status(200)
        .with_body(new_json)
        .create_async()
        .await;

    // アセット v1
    let hash_v1 = sha256_hex(data_v1);
    let hk_v1 = format!("js/main.{}.js", &hash_v1[..8]);
    let _m_v1 = server
        .mock("GET", &format!("/{}", hk_v1) as &str)
        .with_status(200)
        .with_body(data_v1)
        .create_async()
        .await;

    // アセット static
    let hash_static = sha256_hex(data_static);
    let hk_static = format!("js/static.{}.js", &hash_static[..8]);
    let _m_static = server
        .mock("GET", &format!("/{}", hk_static) as &str)
        .with_status(200)
        .with_body(data_static)
        .create_async()
        .await;

    // アセット v2
    let hash_v2 = sha256_hex(data_v2);
    let hk_v2 = format!("js/main.{}.js", &hash_v2[..8]);
    let _m_v2 = server
        .mock("GET", &format!("/{}", hk_v2) as &str)
        .with_status(200)
        .with_body(data_v2)
        .create_async()
        .await;

    let loader = AssetLoader::new(FetchOptions {
        integrity_check: true,
        ..Default::default()
    });
    let config = default_config(vec!["js/main.js", "js/static.js"]);
    let manifest_url = format!("{}/manifest.json", base);

    // 初回ロード（旧 manifest）
    let initial = loader
        .strategy_assets(&manifest_url, &config, None, CancellationToken::new())
        .await
        .expect("initial load should succeed");
    assert_eq!(initial.len(), 2);

    // 差分取得（新 manifest: js/main.js のみ変更）
    let diff_assets = loader
        .fetch_cdn_diff(&manifest_url, None, CancellationToken::new())
        .await
        .expect("cdn diff should succeed");

    assert_eq!(diff_assets.len(), 1);
    assert_eq!(diff_assets[0].key, "js/main.js");
    assert_eq!(diff_assets[0].data, data_v2.as_ref());
}

// ─────────────────────────────────────────────
// Test 4: 進捗コールバック
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_progress_callback_called() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    let files: Vec<(&str, &[u8])> = vec![
        ("js/a.js", b"aaa"),
        ("js/b.js", b"bbb"),
        ("js/c.js", b"ccc"),
    ];

    let manifest = make_manifest(&base, &files);
    let manifest_json = serde_json::to_string(&manifest).unwrap();

    let _m_manifest = server
        .mock("GET", "/manifest.json")
        .with_status(200)
        .with_body(manifest_json)
        .create_async()
        .await;

    for (key, data) in &files {
        let hash_full = sha256_hex(data);
        let hash8 = &hash_full[..8];
        let dot = key.rfind('.').unwrap_or(key.len());
        let (stem, ext) = key.split_at(dot);
        let hashed_key = format!("{}.{}{}", stem, hash8, ext);
        server
            .mock("GET", &format!("/{}", hashed_key) as &str)
            .with_status(200)
            .with_body(*data)
            .create_async()
            .await;
    }

    let progress_count = Arc::new(AtomicUsize::new(0));
    let pc = progress_count.clone();
    let on_progress: Arc<dyn Fn(ProgressEvent) + Send + Sync> =
        Arc::new(move |_: ProgressEvent| {
            pc.fetch_add(1, Ordering::SeqCst);
        });

    let loader = AssetLoader::new(FetchOptions {
        integrity_check: true,
        ..Default::default()
    });
    let config = default_config(vec!["js/a.js", "js/b.js", "js/c.js"]);
    let manifest_url = format!("{}/manifest.json", base);

    loader
        .strategy_assets(
            &manifest_url,
            &config,
            Some(on_progress),
            CancellationToken::new(),
        )
        .await
        .expect("should succeed");

    assert_eq!(progress_count.load(Ordering::SeqCst), 3);
}
