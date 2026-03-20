//! `s3d build` — アセット収集・ハッシュ化・マニフェスト生成コマンド

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use colored::Colorize;
use s3d_deploy::{
    collect::{collect, CollectOptions},
    hash::hash_assets,
    manifest::{build_manifest, manifest_to_json, ManifestOptions},
};

use crate::config::S3dCliConfig;

/// `s3d build` を実行する
pub fn run(config: &S3dCliConfig, config_path: &Path) -> Result<()> {
    let start = Instant::now();
    let project_root = config_path.parent().unwrap_or(Path::new("."));
    let output_dir = project_root.join(&config.output_dir);

    println!("{}", "s3d build — アセットをビルドします".bold().cyan());
    println!("  入力ディレクトリ : {}", output_dir.display());

    // ── 1. 収集
    let collect_opts = CollectOptions {
        ignore: config.exclude.clone(),
        include: config.include.clone(),
        max_file_size: config.max_file_size.clone(),
    };
    let collected = collect(&output_dir, &collect_opts)
        .with_context(|| format!("アセット収集エラー: {}", output_dir.display()))?;
    println!("  収集: {} ファイル", collected.len().to_string().bold());

    // ── 2. ハッシュ化
    let hashed = hash_assets(&collected, s3d_deploy::hash::DEFAULT_HASH_LENGTH)
        .context("ハッシュ計算エラー")?;

    // ── 3. マニフェスト生成
    let manifest_opts = ManifestOptions {
        cdn_base_url: config
            .storage
            .cdn_base_url
            .trim_end_matches('/')
            .to_string(),
        version: "1.0.0".to_string(),
        build_time: Some(chrono::Utc::now().to_rfc3339()),
    };
    let manifest = build_manifest(&hashed, &manifest_opts).context("マニフェスト生成エラー")?;

    // ── 4. manifest.json の書き込み
    let manifest_path = config.resolved_manifest_path();
    let manifest_path = if manifest_path.is_absolute() {
        manifest_path
    } else {
        project_root.join(&manifest_path)
    };
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = manifest_to_json(&manifest).context("マニフェスト JSON 変換エラー")?;
    std::fs::write(&manifest_path, &json)
        .with_context(|| format!("manifest.json の書き込み失敗: {}", manifest_path.display()))?;

    // ── サマリ表示
    let total_size: u64 = hashed.iter().map(|a| a.size).sum();
    let elapsed = start.elapsed();
    println!();
    println!("{}", "ビルド完了 ✔".green().bold());
    println!("  ファイル数   : {}", hashed.len().to_string().bold());
    println!("  合計サイズ   : {}", format_bytes(total_size).bold());
    println!(
        "  manifest.json: {}",
        manifest_path.display().to_string().bold()
    );
    println!("  ビルド時間   : {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    fn make_config(output_dir: &str) -> S3dCliConfig {
        S3dCliConfig {
            project: "test".to_string(),
            storage: StorageConfig {
                provider: CdnProvider::CloudflareR2,
                bucket: "bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: None,
                endpoint: None,
                region: None,
            },
            output_dir: output_dir.to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
        }
    }

    #[test]
    fn test_build_creates_manifest() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        // ダミーアセット
        std::fs::write(output.join("app.js"), b"console.log('hello');").unwrap();
        std::fs::write(output.join("style.css"), b"body { margin: 0; }").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("output");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path).unwrap();

        let manifest_path = dir.path().join("output/manifest.json");
        assert!(manifest_path.exists(), "manifest.json が生成されていない");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(
            content.contains("app.js") || content.contains("app."),
            "app.js がマニフェストに含まれていない"
        );
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1_048_576), "1.00 MB");
    }
}
