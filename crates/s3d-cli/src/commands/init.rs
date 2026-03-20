//! `s3d init` — プロジェクト初期化コマンド
//!
//! インタラクティブなプロンプトで設定を収集し、以下を生成する:
//! - `s3d.config.json`
//! - `.env.example`
//! - `.gitignore`
//! - `src/index.html`                         (スキャフォールド HTML)
//! - `src/assetsStrategy/strategy.json`       (デフォルト配信戦略)
//! - `src/assets/.gitkeep`                    (空ディレクトリ保持)
//! - `output/`                                (ビルド出力先)

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
    };

    save_config(std::path::Path::new("s3d.config.json"), &config)?;
    println!("{}", "✔ s3d.config.json を生成しました".green());

    // ── .env.example
    write_env_example(&provider)?;
    println!("{}", "✔ .env.example を生成しました".green());

    // ── .gitignore
    write_gitignore()?;
    println!("{}", "✔ .gitignore を更新しました".green());

    // ── src/ スキャフォールド
    scaffold_src(&project)?;
    println!("{}", "✔ src/ を生成しました".green());

    // ── output/ ディレクトリ
    std::fs::create_dir_all("output")?;
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
// Scaffold helpers
// ──────────────────────────────────────────────────────────────

fn write_env_example(provider: &CdnProvider) -> Result<()> {
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
    std::fs::write(".env.example", content)?;
    Ok(())
}

fn write_gitignore() -> Result<()> {
    let path = std::path::Path::new(".gitignore");
    let mut content = if path.exists() {
        std::fs::read_to_string(path)?
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
    std::fs::write(path, &content)?;
    Ok(())
}

fn scaffold_src(project: &str) -> Result<()> {
    // src/assets/.gitkeep
    std::fs::create_dir_all("src/assets")?;
    std::fs::write("src/assets/.gitkeep", "")?;

    // src/assetsStrategy/strategy.json
    std::fs::create_dir_all("src/assetsStrategy")?;
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
    std::fs::write("src/assetsStrategy/strategy.json", strategy_json)?;

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
    std::fs::write("src/index.html", &index_html)?;

    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::config::{load_config, save_config, CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    fn make_and_scaffold(project: &str, provider: CdnProvider) -> TempDir {
        let dir = TempDir::new().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

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
        };
        save_config(std::path::Path::new("s3d.config.json"), &config).unwrap();
        super::write_env_example(&provider).unwrap();
        super::write_gitignore().unwrap();
        super::scaffold_src(project).unwrap();
        std::fs::create_dir_all("output").unwrap();

        std::env::set_current_dir(&orig).unwrap();
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
        // ディレクトリ
        assert!(dir.path().join("src").is_dir());
        assert!(dir.path().join("src/assets").is_dir());
        assert!(dir.path().join("src/assetsStrategy").is_dir());
        assert!(dir.path().join("output").is_dir());
        // ファイル
        assert!(dir.path().join("src/index.html").exists());
        assert!(dir.path().join("src/assetsStrategy/strategy.json").exists());
        assert!(dir.path().join("src/assets/.gitkeep").exists());
        assert!(dir.path().join(".env.example").exists());
        assert!(dir.path().join(".gitignore").exists());
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
    fn test_gitignore_contains_output() {
        let dir = make_and_scaffold("proj", CdnProvider::CloudflareR2);
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("output/"));
        assert!(content.contains(".env"));
        assert!(content.contains("/target"));
    }
}
