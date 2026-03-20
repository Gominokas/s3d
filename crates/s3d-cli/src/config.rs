//! s3d.config.json の読み込み・書き込みと `.env` ロード

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────────────────────
// Config structs
// ──────────────────────────────────────────────────────────────

/// CDN プロバイダー
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CdnProvider {
    CloudflareR2,
    AwsS3,
    Custom,
}

impl Default for CdnProvider {
    fn default() -> Self {
        Self::CloudflareR2
    }
}

impl std::fmt::Display for CdnProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CloudflareR2 => write!(f, "cloudflare-r2"),
            Self::AwsS3 => write!(f, "aws-s3"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// ストレージ設定
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub provider: CdnProvider,
    pub bucket: String,
    pub cdn_base_url: String,
    /// R2: アカウントID (Cloudflare R2 使用時)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// カスタムエンドポイント（R2 互換 S3 など）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// AWS リージョン (S3 使用時)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// プロジェクト設定 (s3d.config.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3dCliConfig {
    pub project: String,
    pub storage: StorageConfig,
    /// アセット収集のルートディレクトリ（デフォルト: "output"）
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    /// glob パターン（デフォルト: 空 = 全ファイル）
    #[serde(default)]
    pub include: Vec<String>,
    /// 除外パターン
    #[serde(default)]
    pub exclude: Vec<String>,
    /// 最大ファイルサイズ（例: "100MB"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_size: Option<String>,
    /// manifest.json の出力先（デフォルト: output_dir）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
}

fn default_output_dir() -> String {
    "output".to_string()
}

impl S3dCliConfig {
    /// manifest.json のパスを解決する
    pub fn resolved_manifest_path(&self) -> PathBuf {
        if let Some(p) = &self.manifest_path {
            PathBuf::from(p)
        } else {
            PathBuf::from(&self.output_dir).join("manifest.json")
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Load / Save helpers
// ──────────────────────────────────────────────────────────────

/// `.env` をロードする（ファイルがなくても無視）
pub fn load_dotenv() {
    let _ = dotenvy::dotenv();
}

/// s3d.config.json を読み込む
pub fn load_config(path: &Path) -> Result<S3dCliConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("s3d.config.json が見つかりません: {}", path.display()))?;
    let config: S3dCliConfig =
        serde_json::from_str(&content).context("s3d.config.json のパースに失敗しました")?;
    Ok(config)
}

/// s3d.config.json を保存する
pub fn save_config(path: &Path, config: &S3dCliConfig) -> Result<()> {
    let content =
        serde_json::to_string_pretty(config).context("config の JSON シリアライズに失敗")?;
    std::fs::write(path, content).with_context(|| format!("書き込み失敗: {}", path.display()))?;
    Ok(())
}

/// 設定と環境変数を検証し、不足があればエラーメッセージを返す
pub fn validate_config_and_env(config: &S3dCliConfig) -> Vec<String> {
    let mut errors = Vec::new();

    if config.project.trim().is_empty() {
        errors.push("project が空です".to_string());
    }
    if config.storage.bucket.trim().is_empty() {
        errors.push("storage.bucket が空です".to_string());
    }
    if config.storage.cdn_base_url.trim().is_empty() {
        errors.push("storage.cdn_base_url が空です".to_string());
    }

    // 必須環境変数
    let required_env = match config.storage.provider {
        CdnProvider::CloudflareR2 => vec!["S3D_ACCESS_KEY_ID", "S3D_SECRET_ACCESS_KEY"],
        CdnProvider::AwsS3 => vec!["S3D_ACCESS_KEY_ID", "S3D_SECRET_ACCESS_KEY"],
        CdnProvider::Custom => vec!["S3D_ACCESS_KEY_ID", "S3D_SECRET_ACCESS_KEY"],
    };
    for var in required_env {
        if std::env::var(var).unwrap_or_default().trim().is_empty() {
            errors.push(format!("環境変数 {var} が未設定です"));
        }
    }

    errors
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn sample_config() -> S3dCliConfig {
        S3dCliConfig {
            project: "my-project".to_string(),
            storage: StorageConfig {
                provider: CdnProvider::CloudflareR2,
                bucket: "my-bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: Some("abc123".to_string()),
                endpoint: None,
                region: None,
            },
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
        }
    }

    #[test]
    fn test_save_and_load_config() {
        let tmp = NamedTempFile::new().unwrap();
        let cfg = sample_config();
        save_config(tmp.path(), &cfg).unwrap();
        let loaded = load_config(tmp.path()).unwrap();
        assert_eq!(loaded.project, "my-project");
        assert_eq!(loaded.storage.bucket, "my-bucket");
        assert_eq!(loaded.storage.provider, CdnProvider::CloudflareR2);
    }

    #[test]
    fn test_resolved_manifest_path_default() {
        let cfg = sample_config();
        assert_eq!(
            cfg.resolved_manifest_path(),
            PathBuf::from("output/manifest.json")
        );
    }

    #[test]
    fn test_resolved_manifest_path_custom() {
        let mut cfg = sample_config();
        cfg.manifest_path = Some("dist/manifest.json".to_string());
        assert_eq!(
            cfg.resolved_manifest_path(),
            PathBuf::from("dist/manifest.json")
        );
    }

    #[test]
    fn test_validate_missing_fields() {
        let mut cfg = sample_config();
        cfg.project = "".to_string();
        // env vars not set in test — errors will include env vars too
        let errs = validate_config_and_env(&cfg);
        assert!(errs.iter().any(|e| e.contains("project")));
    }

    #[test]
    fn test_cdn_provider_display() {
        assert_eq!(CdnProvider::CloudflareR2.to_string(), "cloudflare-r2");
        assert_eq!(CdnProvider::AwsS3.to_string(), "aws-s3");
    }
}
