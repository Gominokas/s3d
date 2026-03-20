//! `s3d push` — アセットを R2/S3 へアップロードするコマンド

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use colored::Colorize;
use futures::future::join_all;
use s3d_deploy::diff::{diff_manifests, needs_delete, needs_upload};
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

    let new_manifest = load_local_manifest(&local_manifest_path)?;

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
                    // manifest からハッシュ付きキーを探す（URL から元のキー特定）
                    // ローカルファイルは output_dir/key として読み込む
                    let file_path = output_dir.join(&key);
                    match std::fs::read(&file_path) {
                        Ok(data) => {
                            let content_type = new_manifest
                                .assets
                                .get(&key)
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
                            eprintln!("  {} ファイル読み込み失敗 {key}: {e}", "✘".red());
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

        // マニフェストを手で作る
        let mut assets = HashMap::new();
        assets.insert(
            "app.js".to_string(),
            AssetEntry {
                url: "https://cdn.example.com/app.abcd1234.js".to_string(),
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
                url: "https://cdn.example.com/app.abcd1234.js".to_string(),
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
            assets: assets.clone(),
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

        // ストレージに同じ manifest を事前に入れておく → 差分なし
        let storage = Arc::new(MockStorage::new());
        {
            let manifest_json = serde_json::to_vec_pretty(&manifest).unwrap();
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
}
