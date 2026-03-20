//! `s3d validate` — s3d.config.json / strategy.json / .env の検証コマンド

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::config::{load_config, validate_config_and_env, S3dCliConfig};

/// `s3d validate` を実行する
pub fn run(config_path: &Path) -> Result<()> {
    println!("{}", "s3d validate — 設定の検証".bold().cyan());

    // ── 1. config 読み込み
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

    let project_root = config_path.parent().unwrap_or(Path::new("."));
    let mut all_ok = true;

    // ── 2. config + env 検証
    let errors = validate_config_and_env(&config);
    if errors.is_empty() {
        print_config_success(&config);
    } else {
        all_ok = false;
        for e in &errors {
            println!("  {} {}", "✘".red(), e);
        }
    }

    // ── 3. strategy.json 検証
    let strategy_path = project_root.join(config.strategy_json_path());
    match validate_strategy_json(&strategy_path) {
        Ok(msg) => println!("  {} {}", "✔".green(), msg),
        Err(e) => {
            println!("  {} {}", "✘".red(), e);
            all_ok = false;
        }
    }

    println!();
    if all_ok {
        println!("{}", "すべての検証が通過しました ✔".green().bold());
    } else {
        println!("{}", "検証に失敗しました".red().bold());
    }

    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────

fn print_config_success(config: &S3dCliConfig) {
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
    println!("  {} src_dir       : {}", "✔".green(), config.src_dir);
    println!("  {} output_dir    : {}", "✔".green(), config.output_dir);
    println!(
        "  {} CLOUDFLARE_R2_ACCESS_KEY_ID     : {}",
        "✔".green(),
        mask_env_multi(&["CLOUDFLARE_R2_ACCESS_KEY_ID", "S3D_ACCESS_KEY_ID"])
    );
    println!(
        "  {} CLOUDFLARE_R2_SECRET_ACCESS_KEY : {}",
        "✔".green(),
        mask_env_multi(&["CLOUDFLARE_R2_SECRET_ACCESS_KEY", "S3D_SECRET_ACCESS_KEY"])
    );
}

/// `src/assetsStrategy/strategy.json` を読み込み、必須フィールドを確認する
fn validate_strategy_json(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Err(format!(
            "strategy.json が見つかりません: {} (s3d init で生成できます)",
            path.display()
        ));
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("strategy.json の読み込み失敗: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("strategy.json のパース失敗: {e}"))?;

    let mut missing = Vec::new();
    for key in ["initial", "cdn", "reload"] {
        if v.get(key).is_none() {
            missing.push(key);
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "strategy.json に必須フィールドがありません: {}",
            missing.join(", ")
        ));
    }

    Ok(format!("strategy.json OK ({})", path.display()))
}

fn mask_env_multi(vars: &[&str]) -> String {
    for var in vars {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return if v.len() > 4 {
                    format!("{}****", &v[..4])
                } else {
                    "****".to_string()
                };
            }
        }
    }
    "(未設定)".red().to_string()
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{save_config, CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    fn make_config(dir: &TempDir) -> std::path::PathBuf {
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
            src_dir: "src".to_string(),
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
        };
        save_config(&cfg_path, &cfg).unwrap();
        cfg_path
    }

    #[test]
    fn test_validate_missing_config() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("s3d.config.json");
        run(&cfg_path).unwrap(); // パニックしない
    }

    #[test]
    fn test_validate_missing_strategy() {
        let dir = TempDir::new().unwrap();
        let cfg_path = make_config(&dir);
        // strategy.json がない → エラーメッセージを表示するが panic しない
        run(&cfg_path).unwrap();
    }

    #[test]
    fn test_validate_valid_strategy() {
        let dir = TempDir::new().unwrap();
        let cfg_path = make_config(&dir);

        // strategy.json を生成
        let strategy_dir = dir.path().join("src/assetsStrategy");
        std::fs::create_dir_all(&strategy_dir).unwrap();
        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{"initial":{"sources":[],"cache":true},"cdn":{"files":[],"cache":true,"maxAge":"7d"},"reload":{"trigger":"manifest-change","strategy":"diff"}}"#,
        )
        .unwrap();

        run(&cfg_path).unwrap();
    }

    #[test]
    fn test_strategy_json_missing_fields() {
        let path = std::path::Path::new("/dev/null"); // 空ファイル扱い
                                                      // 空 JSON はパースエラー
        assert!(validate_strategy_json(path).is_err());
    }

    #[test]
    fn test_validate_strategy_json_missing_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("strategy.json");
        std::fs::write(&path, r#"{"initial":{}}"#).unwrap();
        let result = validate_strategy_json(&path);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("cdn") || msg.contains("reload"));
    }

    #[test]
    fn test_validate_strategy_json_ok() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("strategy.json");
        std::fs::write(&path, r#"{"initial":{},"cdn":{},"reload":{}}"#).unwrap();
        assert!(validate_strategy_json(&path).is_ok());
    }
}
