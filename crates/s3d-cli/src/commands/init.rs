//! `s3d init` — プロジェクト初期化コマンド
//!
//! インタラクティブなプロンプトで設定を収集し
//! `s3d.config.json`, `.env.example`, `.gitignore`, `output/` を生成する。

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
            provider,
            bucket,
            cdn_base_url,
            account_id,
            endpoint: None,
            region,
        },
        output_dir: "output".to_string(),
        include: vec![],
        exclude: vec![],
        max_file_size: None,
        manifest_path: None,
    };

    save_config(std::path::Path::new("s3d.config.json"), &config)?;
    println!("{}", "✔ s3d.config.json を生成しました".green());

    // ── .env.example
    std::fs::write(
        ".env.example",
        "S3D_ACCESS_KEY_ID=your_access_key_id\n\
         S3D_SECRET_ACCESS_KEY=your_secret_access_key\n",
    )?;
    println!("{}", "✔ .env.example を生成しました".green());

    // ── .gitignore
    let gitignore_path = std::path::Path::new(".gitignore");
    let mut gitignore_content = if gitignore_path.exists() {
        std::fs::read_to_string(gitignore_path)?
    } else {
        String::new()
    };
    for entry in ["/target", ".env"] {
        if !gitignore_content.contains(entry) {
            if !gitignore_content.ends_with('\n') && !gitignore_content.is_empty() {
                gitignore_content.push('\n');
            }
            gitignore_content.push_str(entry);
            gitignore_content.push('\n');
        }
    }
    std::fs::write(".gitignore", &gitignore_content)?;
    println!("{}", "✔ .gitignore を更新しました".green());

    // ── output/ ディレクトリ
    std::fs::create_dir_all("output")?;
    println!("{}", "✔ output/ ディレクトリを作成しました".green());

    println!();
    println!("{}", "次のステップ:".bold());
    println!("  1. cp .env.example .env  # R2/S3 の認証情報を記入");
    println!("  2. アセットを output/ に配置");
    println!("  3. s3d build              # マニフェスト生成");
    println!("  4. s3d push               # R2/S3 へアップロード");

    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::config::{load_config, save_config, CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    /// init が生成するファイルのテスト（プロンプトなしで直接生成）
    #[test]
    fn test_generate_files() {
        let dir = TempDir::new().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let config = S3dCliConfig {
            project: "test-project".to_string(),
            storage: StorageConfig {
                provider: CdnProvider::CloudflareR2,
                bucket: "test-bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: Some("acc123".to_string()),
                endpoint: None,
                region: None,
            },
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
        };
        save_config(std::path::Path::new("s3d.config.json"), &config).unwrap();

        // .env.example
        std::fs::write(
            ".env.example",
            "S3D_ACCESS_KEY_ID=your_access_key_id\nS3D_SECRET_ACCESS_KEY=your_secret_access_key\n",
        )
        .unwrap();

        // output/
        std::fs::create_dir_all("output").unwrap();

        assert!(std::path::Path::new("s3d.config.json").exists());
        assert!(std::path::Path::new(".env.example").exists());
        assert!(std::path::Path::new("output").is_dir());

        let loaded = load_config(std::path::Path::new("s3d.config.json")).unwrap();
        assert_eq!(loaded.project, "test-project");

        std::env::set_current_dir(&orig).unwrap();
    }
}
