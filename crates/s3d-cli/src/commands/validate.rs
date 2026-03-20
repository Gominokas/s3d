//! `s3d validate` — s3d.config.json / strategy.json / .env の検証コマンド
//!
//! Issue #16 追加: 各サブディレクトリ strategy.json の `files` フィールドに
//! 指定されたファイルが `src/assets/` 内に存在するか確認し、存在しない場合は警告を表示する。

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

    // ── 3. ルート strategy.json 検証 (src/assetsStrategy/strategy.json)
    let strategy_path = project_root.join(config.strategy_json_path());
    match validate_strategy_json(&strategy_path) {
        Ok(msg) => println!("  {} {}", "✔".green(), msg),
        Err(e) => {
            println!("  {} {}", "✘".red(), e);
            all_ok = false;
        }
    }

    // ── 4. サブディレクトリ strategy.json の files 存在確認
    let src_dir = project_root.join(&config.src_dir);
    let strategies_root = src_dir.join("assetsStrategy");
    let assets_dir = src_dir.join("assets");
    let warnings = validate_strategy_files(&strategies_root, &assets_dir);
    for w in &warnings {
        println!("  {} {}", "⚠".yellow(), w);
    }

    println!();
    if all_ok {
        if warnings.is_empty() {
            println!("{}", "すべての検証が通過しました ✔".green().bold());
        } else {
            println!(
                "{}",
                format!("検証は通過しましたが {} 件の警告があります", warnings.len())
                    .yellow()
                    .bold()
            );
        }
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

/// サブディレクトリ戦略の `files` フィールドに指定されたファイルが
/// `src/assets/` 内に存在するか確認する。
///
/// 存在しないファイルは警告メッセージのリストとして返す。
pub fn validate_strategy_files(strategies_root: &Path, assets_dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();

    if !strategies_root.exists() {
        return warnings;
    }

    let entries = match std::fs::read_dir(strategies_root) {
        Ok(e) => e,
        Err(_) => return warnings,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let strategy_file = path.join("strategy.json");
        if !strategy_file.exists() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let content = match std::fs::read_to_string(&strategy_file) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!(
                    "[{}] strategy.json の読み込み失敗: {}",
                    name, e
                ));
                continue;
            }
        };

        let v: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!("[{}] strategy.json のパース失敗: {}", name, e));
                continue;
            }
        };

        if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
            for file_val in files {
                if let Some(file_path) = file_val.as_str() {
                    // assets_dir を起点に相対パスを解決
                    // "assets/sushi.glb" → assets_dir/../assets/sushi.glb を確認
                    // assets_dir は src/assets/ なので、src/ を起点にする
                    let src_dir = assets_dir.parent().unwrap_or(assets_dir);
                    let full_path = src_dir.join(file_path);
                    if !full_path.exists() {
                        warnings.push(format!(
                            "[{}] files に指定されたファイルが src/ に存在しません: {}",
                            name, file_path
                        ));
                    }
                }
            }
        }
    }

    warnings
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
            plugins: vec![],
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

    #[test]
    fn test_validate_strategy_files_no_warnings_when_file_exists() {
        let dir = TempDir::new().unwrap();
        let strategies_root = dir.path().join("assetsStrategy");
        let assets_dir = dir.path().join("assets");
        let sushi_dir = strategies_root.join("sushi");
        std::fs::create_dir_all(&sushi_dir).unwrap();
        std::fs::create_dir_all(&assets_dir).unwrap();

        // ファイルを実際に作成
        std::fs::write(assets_dir.join("sushi.glb"), b"glb").unwrap();

        std::fs::write(
            sushi_dir.join("strategy.json"),
            r#"{"files":["assets/sushi.glb"],"initial":false,"cache":true}"#,
        )
        .unwrap();

        let warnings = validate_strategy_files(&strategies_root, &assets_dir);
        assert!(warnings.is_empty(), "警告が出るべきでない: {:?}", warnings);
    }

    #[test]
    fn test_validate_strategy_files_warns_missing_file() {
        let dir = TempDir::new().unwrap();
        let strategies_root = dir.path().join("assetsStrategy");
        let assets_dir = dir.path().join("assets");
        let sushi_dir = strategies_root.join("sushi");
        std::fs::create_dir_all(&sushi_dir).unwrap();
        std::fs::create_dir_all(&assets_dir).unwrap();

        // assets/sushi.glb は作成しない
        std::fs::write(
            sushi_dir.join("strategy.json"),
            r#"{"files":["assets/sushi.glb"],"initial":false,"cache":true}"#,
        )
        .unwrap();

        let warnings = validate_strategy_files(&strategies_root, &assets_dir);
        assert!(!warnings.is_empty(), "警告が出るべき");
        assert!(
            warnings.iter().any(|w| w.contains("sushi.glb")),
            "sushi.glb の警告がない: {:?}",
            warnings
        );
    }

    #[test]
    fn test_validate_strategy_files_no_strategies_dir() {
        let dir = TempDir::new().unwrap();
        let strategies_root = dir.path().join("assetsStrategy");
        let assets_dir = dir.path().join("assets");
        // assetsStrategy ディレクトリなし → 警告なし
        let warnings = validate_strategy_files(&strategies_root, &assets_dir);
        assert!(warnings.is_empty());
    }
}
