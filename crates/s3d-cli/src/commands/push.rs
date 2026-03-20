//! `s3d push` — アセットを R2/S3 へアップロードするコマンド

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

    // ── R2 から旧 manifest.json を取得
    let old_manifest = fetch_remote_manifest(storage.as_ref(), "manifest.json").await;

    // ── 差分計算
    let entries = diff_manifests(old_manifest.as_ref(), &new_manifest);
    let to_upload = needs_upload(&entries);
    let to_delete = needs_delete(&entries);

    if to_upload.is_empty() && to_delete.is_empty() {
        println!("{}", "変更なし。アップロードは不要です。".dimmed());
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

                    // manifest.assets[key].url からファイルパスを解決する。
                    // rewrite_urls_to_cdn 後の URL 例:
                    //   "https://cdn.example.com/assets/cake-3d.30e14955.bin"
                    // の場合、パス部分 "assets/cake-3d.30e14955.bin" を使う。
                    // 相対 URL "/assets/cake-3d.30e14955.bin" の場合も先頭 '/' を除去して使う。
                    let local_path = entry.and_then(|e| {
                        let url = &e.url;
                        // http(s):// の場合はパス部分のみ取り出す
                        if url.starts_with("http://") || url.starts_with("https://") {
                            url.splitn(4, '/').nth(3).map(|p| p.to_string())
                        } else {
                            // ルート相対 URL ("/assets/...") の場合
                            Some(url.trim_start_matches('/').to_string())
                        }
                    });

                    let file_path = match local_path {
                        Some(ref p) => output_dir.join(p),
                        None => output_dir.join(&key), // フォールバック: 論理キーで探す
                    };

                    match std::fs::read(&file_path) {
                        Ok(data) => {
                            let content_type = entry
                                .map(|e| e.content_type.as_str())
                                .unwrap_or("application/octet-stream");
                            match storage.put(&key, &data, content_type).await {
                                Ok(_) => {
                                    let done = counter
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                        + 1;
                                    eprintln!("  [{done}/{total}] {} {key}", "↑".green());
                                }
                                Err(e) => {
                                    eprintln!("  {} {key}: {}", "✘".red(), e.message);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} ファイル読み込み失敗 {key} ({}): {e}", "✘".red(), file_path.display());
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
        .put("manifest.json", &manifest_json, "application/json")
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
    }
}
