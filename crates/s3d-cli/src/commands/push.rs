//! `s3d push` — アセットを R2/S3 へアップロードするコマンド

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use colored::Colorize;
use futures::future::join_all;
use s3d_deploy::diff::{diff_manifests, needs_delete, needs_upload};
use s3d_deploy::manifest::rewrite_urls_to_cdn;
use s3d_types::manifest::DeployManifest;
use s3d_types::plugin::StoragePlugin;

use crate::config::S3dCliConfig;

/// `s3d push` を実行する
///
/// `dry_run = true` の場合は実際の I/O を行わず差分のみ表示する。
pub async fn run(
    config: &S3dCliConfig,
    config_path: &Path,
    manifest_path_override: Option<&Path>,
    dry_run: bool,
    storage: Arc<dyn StoragePlugin>,
) -> Result<()> {
    println!(
        "{}",
        "s3d push — アセットをアップロードします".bold().cyan()
    );

    let project_root = config_path.parent().unwrap_or(Path::new("."));
    let output_dir = project_root.join(&config.output_dir);

    // ── ローカル manifest.json を読む
    let local_manifest_path = manifest_path_override
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            let mp = config.resolved_manifest_path();
            if mp.is_absolute() {
                mp
            } else {
                project_root.join(&mp)
            }
        });

    let mut new_manifest = load_local_manifest(&local_manifest_path)?;

    // ── ローカルビルド時は相対 URL なので CDN 絶対 URL に書き換える
    let cdn_url = config.storage.cdn_base_url.trim_end_matches('/');
    rewrite_urls_to_cdn(&mut new_manifest, cdn_url);

    // ── loader.js をアップロード（manifest 外の固定ファイル・常時アップロード）
    // `s3d build` が output/loader.js を生成するが manifest には含まれない。
    // 差分計算の対象外のため、変更なし早期 return の前に必ずアップロードする。
    // これにより初回 push でも CDN 上に loader.js が確実に配置される。
    if !dry_run {
        let loader_path = output_dir.join("loader.js");
        if loader_path.exists() {
            match std::fs::read(&loader_path) {
                Ok(data) => {
                    match storage
                        .put_with_cache_control(
                            "loader.js",
                            &data,
                            "application/javascript",
                            Some("no-cache"),
                        )
                        .await
                    {
                        Ok(_) => println!("  {} loader.js をアップロードしました", "↑".green()),
                        Err(e) => eprintln!("  {} loader.js アップロード失敗: {}", "✘".red(), e.message),
                    }
                }
                Err(e) => eprintln!("  {} loader.js 読み込み失敗: {e}", "✘".red()),
            }
        } else {
            eprintln!(
                "  {} output/loader.js が見つかりません（s3d build を先に実行してください）",
                "⚠".yellow()
            );
        }
    }

    // ── R2 から旧 manifest.json を取得
    let old_manifest = fetch_remote_manifest(storage.as_ref(), "manifest.json").await;

    // ── 差分計算
    let entries = diff_manifests(old_manifest.as_ref(), &new_manifest);
    let to_upload = needs_upload(&entries);
    let to_delete = needs_delete(&entries);

    if to_upload.is_empty() && to_delete.is_empty() {
        println!("{}", "変更なし。アセットのアップロードは不要です。".dimmed());
        return Ok(());
    }

    println!(
        "  アップロード: {} ファイル  削除: {} ファイル",
        to_upload.len().to_string().green().bold(),
        to_delete.len().to_string().red().bold(),
    );

    if dry_run {
        println!(
            "{}",
            "[dry-run] 実際のアップロードはスキップします".yellow()
        );
        for key in &to_upload {
            println!("  {} {}", "↑".green(), key);
        }
        for key in &to_delete {
            println!("  {} {}", "✕".red(), key);
        }
        return Ok(());
    }

    // ── アップロード（並列）
    // `to_upload` の各 key は manifest の論理キー（例: "assets/cake-3d.bin"）。
    // build 時にハッシュ付きファイル名でコピーされているため、
    // 実際の output/ 内のファイルは manifest.assets[key].url のパス部分になる。
    // rewrite_urls_to_cdn 後は URL = "https://cdn.example.com/assets/cake-3d.30e14955.bin"
    // → パス部分 "assets/cake-3d.30e14955.bin" でファイルを探す。
    let concurrency = 8usize;
    let upload_keys: Vec<String> = to_upload.iter().map(|s| s.to_string()).collect();
    let total = upload_keys.len();
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    for chunk in upload_keys.chunks(concurrency) {
        let futures: Vec<_> = chunk
            .iter()
            .map(|key| {
                let storage = Arc::clone(&storage);
                let key = key.clone();
                let output_dir = output_dir.clone();
                let new_manifest = new_manifest.clone();
                let counter = Arc::clone(&counter);
                let total = total;

                async move {
                    let entry = new_manifest.assets.get(&key);

                    // manifest.assets[key].url からハッシュ付きパスを取り出す。
                    // このパスは:
                    //   1. ローカルファイルの読み込みパス (output/<path>)
                    //   2. R2 へのアップロードキー
                    // の両方に使う。
                    //
                    // rewrite_urls_to_cdn 後の URL 例:
                    //   "https://cdn.example.com/assets/cake-3d.30e14955.bin"
                    //   → path = "assets/cake-3d.30e14955.bin"
                    // 相対 URL の場合:
                    //   "/assets/cake-3d.30e14955.bin"
                    //   → path = "assets/cake-3d.30e14955.bin"
                    let url_path = entry.and_then(|e| {
                        let url = &e.url;
                        if url.starts_with("http://") || url.starts_with("https://") {
                            // "https://cdn.example.com/assets/foo.hash.bin" → "assets/foo.hash.bin"
                            url.splitn(4, '/').nth(3).map(|p| p.to_string())
                        } else {
                            // "/assets/foo.hash.bin" → "assets/foo.hash.bin"
                            Some(url.trim_start_matches('/').to_string())
                        }
                    });

                    // R2 アップロードキー: URL のパス部分（ハッシュ付き）
                    // フォールバック: 論理キー（ハッシュなし、後方互換）
                    let upload_key = url_path.clone().unwrap_or_else(|| key.clone());
                    let file_path = output_dir.join(&upload_key);

                    match std::fs::read(&file_path) {
                        Ok(data) => {
                            let content_type = entry
                                .map(|e| e.content_type.as_str())
                                .unwrap_or("application/octet-stream");
                            // ファイル種別に応じた Cache-Control を決める（下書き 7）。
                            // 分類に使うのは manifest の情報だけ（key / url / strategies）。
                            let cache_control = cache_control_for(
                                &key,
                                url_path.as_deref(),
                                &new_manifest.strategies,
                            );
                            // storage.put の第1引数はハッシュ付きキー（manifest の URL パス部分）
                            // ブラウザが要求する URL と R2 オブジェクトキーを一致させる
                            match storage
                                .put_with_cache_control(
                                    &upload_key,
                                    &data,
                                    content_type,
                                    cache_control.as_deref(),
                                )
                                .await
                            {
                                Ok(_) => {
                                    let done = counter
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                        + 1;
                                    eprintln!("  [{done}/{total}] {} {upload_key}", "↑".green());
                                }
                                Err(e) => {
                                    eprintln!("  {} {upload_key}: {}", "✘".red(), e.message);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} ファイル読み込み失敗 {upload_key} ({}): {e}", "✘".red(), file_path.display());
                        }
                    }
                }
            })
            .collect();
        join_all(futures).await;
    }

    // ── 削除
    for key in &to_delete {
        match storage.delete(key).await {
            Ok(_) => println!("  {} {}", "✕".red(), key),
            Err(e) => eprintln!("  {} 削除失敗 {key}: {}", "✘".red(), e.message),
        }
    }

    // ── manifest.json をアップロード
    let manifest_json =
        serde_json::to_vec_pretty(&new_manifest).context("manifest JSON 変換失敗")?;
    storage
        .put_with_cache_control(
            "manifest.json",
            &manifest_json,
            "application/json",
            Some("no-cache"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("manifest.json アップロード失敗: {}", e.message))?;
    println!("  {} manifest.json をアップロードしました", "✔".green());

    println!();
    println!(
        "{}  {} ファイルをアップロードしました",
        "push 完了 ✔".green().bold(),
        total.to_string().bold()
    );

    Ok(())
}

fn load_local_manifest(path: &Path) -> Result<DeployManifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("manifest.json が見つかりません: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("manifest.json のパース失敗: {}", path.display()))
}

async fn fetch_remote_manifest(storage: &dyn StoragePlugin, key: &str) -> Option<DeployManifest> {
    match storage.get(key).await {
        Ok(data) => serde_json::from_slice(&data).ok(),
        Err(_) => None, // 初回デプロイ時は None
    }
}

// ──────────────────────────────────────────────────────────────
// Cache-Control 分類（下書き 7）
// ──────────────────────────────────────────────────────────────

/// アップロードするファイルの Cache-Control ヘッダー値を決める。
///
/// 分類は manifest の情報だけ（論理キー / URL パス / strategies）で行う。
///
/// - ハッシュ付きファイル（URL のファイル名が論理キーと違う）:
///   `public, max-age=31536000, immutable`（内容が変われば URL も変わるので長期キャッシュ可）
/// - `initial` のファイル（strategy の initial に載っているパス）:
///   strategy の `maxAge` 宣言があればそれを秒数に変換して使う。なければ短めの既定値。
/// - それ以外（ハッシュなしの通常ファイル）: None（ヘッダー無し・従来どおり）
pub fn cache_control_for(
    key: &str,
    url_path: Option<&str>,
    strategies: &HashMap<String, s3d_types::manifest::StrategyEntry>,
) -> Option<String> {
    // initial のファイルはハッシュなしで、HTML から直接参照される。
    // strategy の maxAge 宣言があればそれを、なければ短めの既定値を使う。
    for entry in strategies.values() {
        if entry.initial.as_deref() == Some(key) {
            let max_age = entry
                .max_age
                .as_deref()
                .and_then(parse_max_age)
                .unwrap_or(3600);
            return Some(format!("public, max-age={max_age}"));
        }
    }

    // URL のファイル名が論理キーのファイル名と違えばハッシュ付き。
    // 例: key "assets/sushi.glb" / url_path "assets/sushi.30e14955.glb"
    let url_file = url_path.and_then(|p| p.rsplit('/').next());
    let key_file = key.rsplit('/').next().unwrap_or(key);
    if let Some(url_file) = url_file {
        if url_file != key_file {
            return Some("public, max-age=31536000, immutable".to_string());
        }
    }

    None
}

/// strategy.json の `maxAge` 宣言（例: "7d" / "1h" / "30m" / "3600"）を秒数に変換する。
///
/// 単位: `s` 秒 / `m` 分 / `h` 時 / `d` 日。単位なしは秒として扱う。
/// パースできない場合は None を返す。
pub fn parse_max_age(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (num_part, multiplier) = match value.chars().last() {
        Some('s') => (&value[..value.len() - 1], 1),
        Some('m') => (&value[..value.len() - 1], 60),
        Some('h') => (&value[..value.len() - 1], 3600),
        Some('d') => (&value[..value.len() - 1], 86400),
        _ => (value, 1),
    };
    num_part
        .trim()
        .parse::<u64>()
        .ok()
        .map(|n| n.saturating_mul(multiplier))
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use s3d_types::plugin::{StorageError, StorageResult};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// インメモリのモックストレージ
    struct MockStorage {
        data: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl StoragePlugin for MockStorage {
        async fn put(&self, key: &str, data: &[u8], _ct: &str) -> StorageResult<()> {
            self.data
                .lock()
                .unwrap()
                .insert(key.to_string(), data.to_vec());
            Ok(())
        }
        async fn get(&self, key: &str) -> StorageResult<Vec<u8>> {
            self.data
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| StorageError {
                    message: "not found".into(),
                    key: Some(key.into()),
                })
        }
        async fn delete(&self, key: &str) -> StorageResult<()> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }
        async fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
            let keys: Vec<String> = self
                .data
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect();
            Ok(keys)
        }
    }

    fn make_config() -> S3dCliConfig {
        crate::config::S3dCliConfig {
            project: "test".to_string(),
            storage: crate::config::StorageConfig {
                provider: crate::config::CdnProvider::CloudflareR2,
                bucket: "bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: None,
                endpoint: None,
                region: None,
            },
            src_dir: "src".to_string(),
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
            plugins: vec![],
        }
    }

    #[tokio::test]
    async fn test_push_dry_run() {
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("app.js"), b"console.log(1);").unwrap();

        // ビルドが生成するルート相対 URL を持つマニフェスト
        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "/app.abcd1234.js".to_string(), // 相対 URL
                size: 16,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets,
            strategies: HashMap::new(),
        };
        let manifest_path = dir.path().join("output/manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config();
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        let storage: Arc<dyn StoragePlugin> = Arc::new(MockStorage::new());
        run(&cfg, &cfg_path, Some(&manifest_path), true, storage)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_push_no_changes() {
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "/app.abcd1234.js".to_string(), // 相対 URL
                size: 16,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let relative_manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: assets.clone(),
            strategies: HashMap::new(),
        };
        let manifest_path = dir.path().join("output/manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&relative_manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config();
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        // ストレージには CDN 絶対 URL を持つ manifest が入っている（push 済み状態）
        let mut cdn_assets = HashMap::new();
        cdn_assets.insert(
            "app.js".to_string(),
            s3d_types::manifest::AssetEntry {
                url: "https://cdn.example.com/app.abcd1234.js".to_string(),
                size: 16,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let cdn_manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: cdn_assets,
            strategies: HashMap::new(),
        };
        let storage = Arc::new(MockStorage::new());
        {
            let manifest_json = serde_json::to_vec_pretty(&cdn_manifest).unwrap();
            storage
                .put("manifest.json", &manifest_json, "application/json")
                .await
                .unwrap();
        }
        let storage: Arc<dyn StoragePlugin> = storage;

        run(
            &cfg,
            &cfg_path,
            Some(&manifest_path),
            false,
            Arc::clone(&storage),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_push_rewrites_relative_urls_to_cdn() {
        // push 時にルート相対 URL → CDN 絶対 URL に書き換えられることを確認
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("app.abcd1234.js"), b"console.log(1);").unwrap();

        // ビルドが生成したルート相対 URL を持つマニフェスト
        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "/app.abcd1234.js".to_string(), // 相対 URL
                size: 15,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let relative_manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets,
            strategies: HashMap::new(),
        };
        let manifest_path = dir.path().join("output/manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&relative_manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config(); // cdn_base_url = "https://cdn.example.com"
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        let storage = Arc::new(MockStorage::new());
        let storage: Arc<dyn StoragePlugin> = storage.clone();

        // dry-run=false で実行（アップロードする）
        run(&cfg, &cfg_path, Some(&manifest_path), false, Arc::clone(&storage))
            .await
            .unwrap();

        // アップロードされた manifest.json を取得して URL を確認
        let uploaded_manifest_bytes = storage.get("manifest.json").await.unwrap();
        let uploaded_manifest: DeployManifest =
            serde_json::from_slice(&uploaded_manifest_bytes).unwrap();

        let url = &uploaded_manifest.assets["app.js"].url;
        assert!(
            url.starts_with("https://cdn.example.com/"),
            "push 後の manifest.json の URL は CDN 絶対 URL であるべき: {url}"
        );
        assert_eq!(url, "https://cdn.example.com/app.abcd1234.js");

        // R2 にはハッシュ付きキーで保存されている（論理キーではない）
        let r2_data = storage.get("app.abcd1234.js").await
            .expect("R2 のキーはハッシュ付き app.abcd1234.js であるべき");
        assert_eq!(r2_data, b"console.log(1);");
        // 論理キーでは保存されていない
        assert!(storage.get("app.js").await.is_err(),
            "論理キー app.js で R2 に保存されていてはならない");
    }

    #[tokio::test]
    async fn test_push_uploads_loader_js() {
        // output/loader.js が存在するとき push がそれをアップロードすることを確認
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        // loader.js を配置（build が生成するものを模倣）
        let loader_content = b"export{strategyAssets}";
        std::fs::write(output.join("loader.js"), loader_content).unwrap();

        // ハッシュ付きアセットファイルも配置
        std::fs::write(output.join("app.abcd1234.js"), b"console.log(1);").unwrap();

        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "/app.abcd1234.js".to_string(),
                size: 15,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets,
            strategies: HashMap::new(),
        };
        let manifest_path = output.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config();
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        let storage = Arc::new(MockStorage::new());
        let storage_dyn: Arc<dyn StoragePlugin> = storage.clone();

        run(&cfg, &cfg_path, Some(&manifest_path), false, Arc::clone(&storage_dyn))
            .await
            .unwrap();

        // loader.js がアップロードされている
        let uploaded = storage.get("loader.js").await
            .expect("loader.js がストレージにアップロードされていない");
        assert_eq!(uploaded, loader_content, "loader.js の内容が一致しない");

        // アセットはハッシュ付きキーで R2 に保存されている
        let asset_data = storage.get("app.abcd1234.js").await
            .expect("R2 のキーはハッシュ付き app.abcd1234.js であるべき");
        assert_eq!(asset_data, b"console.log(1);");
        assert!(storage.get("app.js").await.is_err(),
            "論理キー app.js で R2 に保存されていてはならない");
    }

    #[tokio::test]
    async fn test_push_no_changes_still_uploads_loader_js() {
        // アセットに変更がない（no changes）ときでも loader.js はアップロードされる
        // loader.js は差分計算の対象外のため、変更なし early return の前に必ずアップロードする
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        let loader_content = b"export{strategyAssets}";
        std::fs::write(output.join("loader.js"), loader_content).unwrap();

        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "/app.abcd1234.js".to_string(),
                size: 16,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let relative_manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: assets.clone(),
            strategies: HashMap::new(),
        };
        let manifest_path = output.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&relative_manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config();
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        // リモートに CDN URL で同じ内容の manifest が既にある（アセット変更なし）
        let mut cdn_assets = HashMap::new();
        cdn_assets.insert(
            "app.js".to_string(),
            s3d_types::manifest::AssetEntry {
                url: "https://cdn.example.com/app.abcd1234.js".to_string(),
                size: 16,
                hash: "abcd1234".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        let cdn_manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: cdn_assets,
            strategies: HashMap::new(),
        };
        let storage = Arc::new(MockStorage::new());
        {
            let manifest_json = serde_json::to_vec_pretty(&cdn_manifest).unwrap();
            storage
                .put("manifest.json", &manifest_json, "application/json")
                .await
                .unwrap();
        }
        let storage_dyn: Arc<dyn StoragePlugin> = storage.clone();

        // アセット変更なしでも run は成功する
        run(&cfg, &cfg_path, Some(&manifest_path), false, Arc::clone(&storage_dyn))
            .await
            .unwrap();

        // loader.js は変更なし early return の前にアップロードされている
        let uploaded = storage.get("loader.js").await
            .expect("変更なし時でも loader.js はアップロードされるべき");
        assert_eq!(uploaded, loader_content, "loader.js の内容が一致しない");
    }

    #[tokio::test]
    async fn test_push_uses_hashed_key_for_r2() {
        // Issue #36: R2 アップロードキーが論理キーではなくハッシュ付きパスになることを確認
        // storage.put(&key, ...) → storage.put(&upload_key, ...) の修正を検証
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        let assets_dir = output.join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        // ハッシュ付きファイルを output/ に配置（s3d build が生成する状態を模倣）
        let glb_data = b"glb-binary-data";
        std::fs::write(assets_dir.join("cake-3d.30e14955.bin"), glb_data).unwrap();

        let mut assets = HashMap::new();
        assets.insert(
            "assets/cake-3d.bin".to_string(), // 論理キー
            AssetEntry {
                url: "/assets/cake-3d.30e14955.bin".to_string(), // ハッシュ付き相対 URL
                size: glb_data.len() as u64,
                hash: "30e14955".to_string(),
                content_type: "application/octet-stream".to_string(),
                dependencies: None,
            },
        );
        let manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets,
            strategies: HashMap::new(),
        };
        let manifest_path = output.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = make_config(); // cdn_base_url = "https://cdn.example.com"
        crate::config::save_config(&cfg_path, &cfg).unwrap();

        let storage = Arc::new(MockStorage::new());
        let storage_dyn: Arc<dyn StoragePlugin> = storage.clone();

        run(&cfg, &cfg_path, Some(&manifest_path), false, Arc::clone(&storage_dyn))
            .await
            .unwrap();

        // ハッシュ付きキーで R2 にアップロードされている
        let r2_data = storage.get("assets/cake-3d.30e14955.bin").await
            .expect("R2 のキーはハッシュ付き assets/cake-3d.30e14955.bin であるべき");
        assert_eq!(r2_data, glb_data);

        // 論理キーでは保存されていない（ブラウザが参照する URL と異なるため）
        assert!(
            storage.get("assets/cake-3d.bin").await.is_err(),
            "論理キー assets/cake-3d.bin で R2 に保存されていてはならない"
        );

        // manifest.json の URL が CDN 絶対 URL になっている
        let manifest_bytes = storage.get("manifest.json").await.unwrap();
        let uploaded_manifest: DeployManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        let url = &uploaded_manifest.assets["assets/cake-3d.bin"].url;
        assert_eq!(
            url,
            "https://cdn.example.com/assets/cake-3d.30e14955.bin",
            "manifest の URL が CDN 絶対 URL + ハッシュ付きであるべき"
        );
    }

    // ── Cache-Control 分類テスト（下書き 7） ─────────────────────────────

    fn strategy_with_initial(initial: &str, max_age: Option<&str>) -> s3d_types::manifest::StrategyEntry {
        s3d_types::manifest::StrategyEntry {
            files: vec![],
            initial: Some(initial.to_string()),
            cache: true,
            max_age: max_age.map(|s| s.to_string()),
            reload: None,
        }
    }

    #[test]
    fn test_cache_control_for_hashed_file() {
        let strategies = HashMap::new();
        let cc = cache_control_for(
            "assets/sushi.glb",
            Some("assets/sushi.30e14955.glb"),
            &strategies,
        );
        assert_eq!(
            cc.as_deref(),
            Some("public, max-age=31536000, immutable"),
            "ハッシュ付きファイルは長期キャッシュ + immutable であるべき"
        );
    }

    #[test]
    fn test_cache_control_for_unhashed_file() {
        let strategies = HashMap::new();
        let cc = cache_control_for(
            "assets/style.css",
            Some("assets/style.css"),
            &strategies,
        );
        assert_eq!(cc, None, "ハッシュなしの通常ファイルはヘッダー無し（従来どおり）");
    }

    #[test]
    fn test_cache_control_for_initial_with_max_age() {
        let mut strategies = HashMap::new();
        strategies.insert(
            "sushi".to_string(),
            strategy_with_initial("assets/placeholder.png", Some("7d")),
        );
        let cc = cache_control_for(
            "assets/placeholder.png",
            Some("assets/placeholder.png"),
            &strategies,
        );
        assert_eq!(
            cc.as_deref(),
            Some("public, max-age=604800"),
            "initial は strategy の maxAge（7d = 604800秒）を使うべき"
        );
    }

    #[test]
    fn test_cache_control_for_initial_without_max_age() {
        let mut strategies = HashMap::new();
        strategies.insert(
            "sushi".to_string(),
            strategy_with_initial("assets/placeholder.png", None),
        );
        let cc = cache_control_for(
            "assets/placeholder.png",
            Some("assets/placeholder.png"),
            &strategies,
        );
        assert_eq!(
            cc.as_deref(),
            Some("public, max-age=3600"),
            "initial は maxAge 宣言がなければ短めの既定値（3600秒）を使うべき"
        );
    }

    #[test]
    fn test_parse_max_age() {
        assert_eq!(parse_max_age("7d"), Some(604800));
        assert_eq!(parse_max_age("1h"), Some(3600));
        assert_eq!(parse_max_age("30m"), Some(1800));
        assert_eq!(parse_max_age("3600"), Some(3600));
        assert_eq!(parse_max_age("90s"), Some(90));
        assert_eq!(parse_max_age("abc"), None);
        assert_eq!(parse_max_age(""), None);
    }
}
