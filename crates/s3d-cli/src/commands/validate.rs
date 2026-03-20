//! `s3d validate` — s3d.config.json と .env の検証コマンド

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::config::{load_config, validate_config_and_env, S3dCliConfig};

/// `s3d validate` を実行する
pub fn run(config_path: &Path) -> Result<()> {
    println!("{}", "s3d validate — 設定の検証".bold().cyan());

    // ── config 読み込み
    let config = match load_config(config_path) {
        Ok(c) => {
            println!("  {} s3d.config.json を読み込みました", "✔".green());
            c
        }
        Err(e) => {
            println!("  {} {}", "✘".red(), e);
            return Ok(());
        }
    };

    // ── 検証
    let errors = validate_config_and_env(&config);

    if errors.is_empty() {
        print_success(&config);
    } else {
        for e in &errors {
            println!("  {} {}", "✘".red(), e);
        }
        println!();
        println!("{}", "検証に失敗しました".red().bold());
    }

    Ok(())
}

fn print_success(config: &S3dCliConfig) {
    println!("  {} project       : {}", "✔".green(), config.project);
    println!(
        "  {} provider      : {}",
        "✔".green(),
        config.storage.provider
    );
    println!(
        "  {} bucket        : {}",
        "✔".green(),
        config.storage.bucket
    );
    println!(
        "  {} cdn_base_url  : {}",
        "✔".green(),
        config.storage.cdn_base_url
    );
    println!("  {} output_dir    : {}", "✔".green(), config.output_dir);
    println!(
        "  {} S3D_ACCESS_KEY_ID     : {}",
        "✔".green(),
        mask_env("S3D_ACCESS_KEY_ID")
    );
    println!(
        "  {} S3D_SECRET_ACCESS_KEY : {}",
        "✔".green(),
        mask_env("S3D_SECRET_ACCESS_KEY")
    );
    println!();
    println!("{}", "すべての検証が通過しました ✔".green().bold());
}

fn mask_env(var: &str) -> String {
    match std::env::var(var) {
        Ok(v) if v.len() > 4 => format!("{}****", &v[..4]),
        Ok(v) if !v.is_empty() => "****".to_string(),
        _ => "(未設定)".red().to_string(),
    }
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{save_config, CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    #[test]
    fn test_validate_missing_config() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("s3d.config.json");
        // ファイルが存在しない場合はエラーではなく "✘" を表示するだけ
        run(&cfg_path).unwrap();
    }

    #[test]
    fn test_validate_valid_config() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("s3d.config.json");
        let cfg = S3dCliConfig {
            project: "proj".to_string(),
            storage: StorageConfig {
                provider: CdnProvider::CloudflareR2,
                bucket: "bkt".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: None,
                endpoint: None,
                region: None,
            },
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
        };
        save_config(&cfg_path, &cfg).unwrap();
        // 環境変数なしでも run は panic しない
        run(&cfg_path).unwrap();
    }
}
