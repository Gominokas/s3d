//! `s3d init` — プロジェクト初期化コマンド
//!
//! インタラクティブなプロンプトで設定を収集し、以下を生成する:
//! - `s3d.config.json`
//! - `.env.example`
//! - `.gitignore`
//! - `src/index.html`                         (スキャフォールド HTML)
//! - `src/assetsStrategy/strategy.json`       (デフォルト配信戦略)
//! - `src/assetsStrategy/sushi/strategy.json` (サブディレクトリ戦略の例)
//! - `src/assets/.gitkeep`                    (空ディレクトリ保持)
//! - `output/`                                (ビルド出力先)

use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use dialoguer::{Input, Select};

use crate::config::{save_config, CdnProvider, S3dCliConfig, StorageConfig};

const PROVIDERS: &[&str] = &["cloudflare-r2", "aws-s3", "custom"];

/// `s3d init` を実行する
pub fn run() -> Result<()> {
    println!("{}", "s3d init — プロジェクトの初期化".bold().cyan());
    println!();

    // ── プロジェクト名
    let project: String = Input::new()
        .with_prompt("プロジェクト名")
        .default("my-project".to_string())
        .interact_text()?;

    // ── CDN プロバイダー
    let provider_idx = Select::new()
        .with_prompt("CDN プロバイダー")
        .items(PROVIDERS)
        .default(0)
        .interact()?;
    let provider = match provider_idx {
        1 => CdnProvider::AwsS3,
        2 => CdnProvider::Custom,
        _ => CdnProvider::CloudflareR2,
    };

    // ── バケット名
    let bucket: String = Input::new().with_prompt("バケット名").interact_text()?;

    // ── CDN ベース URL
    let cdn_base_url: String = Input::new()
        .with_prompt("CDN ベース URL (例: https://cdn.example.com)")
        .interact_text()?;

    // ── アカウント ID (R2 のみ)
    let account_id = if provider == CdnProvider::CloudflareR2 {
        let id: String = Input::new()
            .with_prompt("Cloudflare アカウント ID (省略可)")
            .allow_empty(true)
            .interact_text()?;
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    } else {
        None
    };

    // ── リージョン (S3 のみ)
    let region = if provider == CdnProvider::AwsS3 {
        let r: String = Input::new()
            .with_prompt("AWS リージョン (例: ap-northeast-1)")
            .default("us-east-1".to_string())
            .interact_text()?;
        Some(r)
    } else {
        None
    };

    // ── カレントディレクトリを base_dir として使用（set_current_dir 不使用）
    let base_dir = std::env::current_dir()?;

    // ── 設定ファイルを生成
    let config = S3dCliConfig {
        project: project.clone(),
        storage: StorageConfig {
            provider: provider.clone(),
            bucket,
            cdn_base_url,
            account_id,
            endpoint: None,
            region,
        },
        src_dir: "src".to_string(),
        output_dir: "output".to_string(),
        include: vec![],
        exclude: vec![],
        max_file_size: None,
        manifest_path: None,
        plugins: vec![],
    };

    save_config(&base_dir.join("s3d.config.json"), &config)?;
    println!("{}", "✔ s3d.config.json を生成しました".green());

    // ── .env.example
    write_env_example(&provider, &base_dir)?;
    println!("{}", "✔ .env.example を生成しました".green());

    // ── .gitignore
    write_gitignore(&base_dir)?;
    println!("{}", "✔ .gitignore を更新しました".green());

    // ── src/ スキャフォールド
    scaffold_src(&project, &base_dir)?;
    println!("{}", "✔ src/ を生成しました".green());

    // ── output/ ディレクトリ
    std::fs::create_dir_all(base_dir.join("output"))?;
    println!("{}", "✔ output/ ディレクトリを作成しました".green());

    println!();
    println!("{}", "次のステップ:".bold());
    println!("  1. cp .env.example .env  # 認証情報を記入");
    println!("  2. src/ にファイルを配置");
    println!("  3. s3d build              # マニフェスト生成");
    println!("  4. s3d push               # R2/S3 へアップロード");

    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Scaffold helpers — すべて base_dir を受け取り、cwd に依存しない
// ──────────────────────────────────────────────────────────────

pub(crate) fn write_env_example(provider: &CdnProvider, base_dir: &Path) -> Result<()> {
    let content = match provider {
        CdnProvider::CloudflareR2 => {
            "CLOUDFLARE_ACCOUNT_ID=your_cloudflare_account_id\n\
             CLOUDFLARE_R2_ACCESS_KEY_ID=your_r2_access_key_id\n\
             CLOUDFLARE_R2_SECRET_ACCESS_KEY=your_r2_secret_access_key\n"
        }
        CdnProvider::AwsS3 | CdnProvider::Custom => {
            "S3D_ACCESS_KEY_ID=your_access_key_id\n\
             S3D_SECRET_ACCESS_KEY=your_secret_access_key\n"
        }
    };
    std::fs::write(base_dir.join(".env.example"), content)?;
    Ok(())
}

pub(crate) fn write_gitignore(base_dir: &Path) -> Result<()> {
    let path = base_dir.join(".gitignore");
    let mut content = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    for entry in ["/target", ".env", "output/"] {
        if !content.contains(entry) {
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(entry);
            content.push('\n');
        }
    }
    std::fs::write(&path, &content)?;
    Ok(())
}

pub(crate) fn scaffold_src(project: &str, base_dir: &Path) -> Result<()> {
    // src/assets/.gitkeep
    let assets_dir = base_dir.join("src/assets");
    std::fs::create_dir_all(&assets_dir)?;
    std::fs::write(assets_dir.join(".gitkeep"), "")?;

    // src/assetsStrategy/strategy.json (ルートレベルのデフォルト戦略)
    let strategy_dir = base_dir.join("src/assetsStrategy");
    std::fs::create_dir_all(&strategy_dir)?;
    let strategy_json = r#"{
  "initial": {
    "sources": ["assets/style.css", "assets/main.js", "assets/hero.png"],
    "cache": true
  },
  "cdn": {
    "files": ["assets/models/**", "assets/detail-*.png"],
    "cache": true,
    "maxAge": "7d"
  },
  "reload": {
    "trigger": "manifest-change",
    "strategy": "diff"
  }
}
"#;
    std::fs::write(strategy_dir.join("strategy.json"), strategy_json)?;

    // src/assetsStrategy/sushi/strategy.json (サブディレクトリ戦略の例)
    // フォルダ名 = strategyAssets("sushi") の呼び出し名と一致
    let sushi_dir = strategy_dir.join("sushi");
    std::fs::create_dir_all(&sushi_dir)?;
    let sushi_strategy_json = r#"{
  "files": ["assets/sushi.glb"],
  "initial": false,
  "cache": true,
  "maxAge": "7d",
  "reload": {
    "trigger": "manifest-change",
    "strategy": "diff"
  }
}
"#;
    std::fs::write(sushi_dir.join("strategy.json"), sushi_strategy_json)?;

    // src/index.html
    let index_html = format!(
        r#"<!DOCTYPE html>
<html lang="ja">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{project}</title>
  <link rel="stylesheet" href="assets/style.css" />
</head>
<body>
  <h1>{project}</h1>

  <!--
    SEO 対象のアセットは直接 <img>/<link> で参照する:
    <img src="assets/hero.png" alt="hero" />

    SEO 不要の重いアセットは strategyAssets() で非同期取得する:
    <script type="module">
      const {{ strategyAssets }} = await import('./assetsStrategy/loader.js');
      const assets = await strategyAssets();
      // assets.get('assets/models/shop.glb') → CDN URL
    </script>
  -->

  <script src="assets/main.js"></script>
</body>
</html>
"#,
        project = project
    );
    std::fs::write(base_dir.join("src/index.html"), &index_html)?;

    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Tests — set_current_dir を一切使わない
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{scaffold_src, write_env_example, write_gitignore};
    use crate::config::{load_config, save_config, CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    /// TempDir を base_dir として直接渡してスキャフォールドを生成する。
    /// set_current_dir は使わない。
    fn make_and_scaffold(project: &str, provider: CdnProvider) -> TempDir {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let config = S3dCliConfig {
            project: project.to_string(),
            storage: StorageConfig {
                provider: provider.clone(),
                bucket: "test-bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: Some("acc123".to_string()),
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
        save_config(&base.join("s3d.config.json"), &config).unwrap();
        write_env_example(&provider, base).unwrap();
        write_gitignore(base).unwrap();
        scaffold_src(project, base).unwrap();
        std::fs::create_dir_all(base.join("output")).unwrap();

        dir
    }

    #[test]
    fn test_generate_config_file() {
        let dir = make_and_scaffold("test-project", CdnProvider::CloudflareR2);
        let loaded = load_config(&dir.path().join("s3d.config.json")).unwrap();
        assert_eq!(loaded.project, "test-project");
        assert_eq!(loaded.src_dir, "src");
        assert_eq!(loaded.output_dir, "output");
    }

    #[test]
    fn test_scaffold_src_structure() {
        let dir = make_and_scaffold("myapp", CdnProvider::CloudflareR2);
        let base = dir.path();
        assert!(base.join("src").is_dir());
        assert!(base.join("src/assets").is_dir());
        assert!(base.join("src/assetsStrategy").is_dir());
        assert!(base.join("output").is_dir());
        assert!(base.join("src/index.html").exists());
        assert!(base.join("src/assetsStrategy/strategy.json").exists());
        assert!(base.join("src/assets/.gitkeep").exists());
        assert!(base.join(".env.example").exists());
        assert!(base.join(".gitignore").exists());
    }

    #[test]
    fn test_index_html_contains_project_name() {
        let dir = make_and_scaffold("MyShop3D", CdnProvider::CloudflareR2);
        let html = std::fs::read_to_string(dir.path().join("src/index.html")).unwrap();
        assert!(html.contains("MyShop3D"));
    }

    #[test]
    fn test_strategy_json_valid() {
        let dir = make_and_scaffold("proj", CdnProvider::CloudflareR2);
        let content =
            std::fs::read_to_string(dir.path().join("src/assetsStrategy/strategy.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(v.get("initial").is_some());
        assert!(v.get("cdn").is_some());
        assert!(v.get("reload").is_some());
    }

    #[test]
    fn test_env_example_cloudflare() {
        let dir = make_and_scaffold("proj", CdnProvider::CloudflareR2);
        let content = std::fs::read_to_string(dir.path().join(".env.example")).unwrap();
        assert!(content.contains("CLOUDFLARE_R2_ACCESS_KEY_ID"));
        assert!(content.contains("CLOUDFLARE_R2_SECRET_ACCESS_KEY"));
        assert!(content.contains("CLOUDFLARE_ACCOUNT_ID"));
    }

    #[test]
    fn test_env_example_aws() {
        let dir = make_and_scaffold("proj", CdnProvider::AwsS3);
        let content = std::fs::read_to_string(dir.path().join(".env.example")).unwrap();
        assert!(content.contains("S3D_ACCESS_KEY_ID"));
        assert!(content.contains("S3D_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn test_gitignore_contains_output() {
        let dir = make_and_scaffold("proj", CdnProvider::CloudflareR2);
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("output/"));
        assert!(content.contains(".env"));
        assert!(content.contains("/target"));
    }

    #[test]
    fn test_gitignore_no_duplicate_entries() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        // 既存の .gitignore に /target と .env が既にある場合
        std::fs::write(base.join(".gitignore"), "/target\n.env\n").unwrap();
        write_gitignore(base).unwrap();
        let content = std::fs::read_to_string(base.join(".gitignore")).unwrap();
        // /target は1回だけ現れる
        assert_eq!(content.matches("/target").count(), 1);
        // .env は1回だけ現れる
        assert_eq!(content.matches(".env\n").count(), 1);
        // output/ は追記される
        assert!(content.contains("output/"));
    }

    #[test]
    fn test_scaffold_independent_dirs() {
        // 2つのテストが同時に異なるディレクトリで動いても競合しないことを確認
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();

        scaffold_src("proj-a", dir_a.path()).unwrap();
        scaffold_src("proj-b", dir_b.path()).unwrap();

        let html_a = std::fs::read_to_string(dir_a.path().join("src/index.html")).unwrap();
        let html_b = std::fs::read_to_string(dir_b.path().join("src/index.html")).unwrap();

        assert!(html_a.contains("proj-a"));
        assert!(html_b.contains("proj-b"));
        // 互いに干渉していない
        assert!(!html_a.contains("proj-b"));
        assert!(!html_b.contains("proj-a"));
    }

    #[test]
    fn test_scaffold_sushi_strategy_exists() {
        // サブディレクトリ戦略 (sushi) が生成されていることを確認
        let dir = TempDir::new().unwrap();
        scaffold_src("test", dir.path()).unwrap();
        let sushi_path = dir.path().join("src/assetsStrategy/sushi/strategy.json");
        assert!(sushi_path.exists(), "sushi/strategy.json が生成されていない");
        let content = std::fs::read_to_string(&sushi_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(v.get("files").is_some(), "files フィールドがない");
        assert!(v.get("initial").is_some(), "initial フィールドがない");
        assert!(v.get("cache").is_some(), "cache フィールドがない");
        let files = v["files"].as_array().unwrap();
        assert!(!files.is_empty(), "files が空");
    }

    #[test]
    fn test_config_plugins_field() {
        // plugins フィールドが空配列で生成されることを確認
        let dir = make_and_scaffold("plugtest", CdnProvider::CloudflareR2);
        let loaded =
            crate::config::load_config(&dir.path().join("s3d.config.json")).unwrap();
        assert!(loaded.plugins.is_empty(), "plugins は空配列であるべき");
    }
}
