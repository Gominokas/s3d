//! 認証情報ヘルパー
//!
//! 環境変数から R2/S3 アクセスキーを読み取る。
//!
//! Cloudflare R2 向け推奨変数:
//!   `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_R2_ACCESS_KEY_ID`, `CLOUDFLARE_R2_SECRET_ACCESS_KEY`
//!
//! 後方互換のフォールバック:
//!   `S3D_ACCESS_KEY_ID`, `S3D_SECRET_ACCESS_KEY`

use anyhow::{Context, Result};

/// R2/S3 の認証情報
#[derive(Debug, Clone)]
pub struct StorageCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl StorageCredentials {
    /// 環境変数から認証情報を読み込む。
    ///
    /// 優先順:
    /// 1. `CLOUDFLARE_R2_ACCESS_KEY_ID` / `CLOUDFLARE_R2_SECRET_ACCESS_KEY`
    /// 2. `S3D_ACCESS_KEY_ID` / `S3D_SECRET_ACCESS_KEY`
    pub fn from_env() -> Result<Self> {
        let access_key_id =
            read_env_with_fallback("CLOUDFLARE_R2_ACCESS_KEY_ID", "S3D_ACCESS_KEY_ID").context(
                "環境変数 CLOUDFLARE_R2_ACCESS_KEY_ID (または S3D_ACCESS_KEY_ID) が未設定です",
            )?;

        let secret_access_key = read_env_with_fallback(
            "CLOUDFLARE_R2_SECRET_ACCESS_KEY",
            "S3D_SECRET_ACCESS_KEY",
        )
        .context(
            "環境変数 CLOUDFLARE_R2_SECRET_ACCESS_KEY (または S3D_SECRET_ACCESS_KEY) が未設定です",
        )?;

        Ok(Self {
            access_key_id,
            secret_access_key,
        })
    }
}

/// primary を優先し、未設定なら fallback を読む
fn read_env_with_fallback(primary: &str, fallback: &str) -> Option<String> {
    let v = std::env::var(primary).unwrap_or_default();
    if !v.trim().is_empty() {
        return Some(v);
    }
    let v = std::env::var(fallback).unwrap_or_default();
    if !v.trim().is_empty() {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_missing() {
        std::env::remove_var("CLOUDFLARE_R2_ACCESS_KEY_ID");
        std::env::remove_var("CLOUDFLARE_R2_SECRET_ACCESS_KEY");
        std::env::remove_var("S3D_ACCESS_KEY_ID");
        std::env::remove_var("S3D_SECRET_ACCESS_KEY");
        assert!(StorageCredentials::from_env().is_err());
    }

    #[test]
    fn test_from_env_cloudflare_vars() {
        std::env::set_var("CLOUDFLARE_R2_ACCESS_KEY_ID", "cfkey");
        std::env::set_var("CLOUDFLARE_R2_SECRET_ACCESS_KEY", "cfsecret");
        std::env::remove_var("S3D_ACCESS_KEY_ID");
        std::env::remove_var("S3D_SECRET_ACCESS_KEY");

        let creds = StorageCredentials::from_env().unwrap();
        assert_eq!(creds.access_key_id, "cfkey");
        assert_eq!(creds.secret_access_key, "cfsecret");

        std::env::remove_var("CLOUDFLARE_R2_ACCESS_KEY_ID");
        std::env::remove_var("CLOUDFLARE_R2_SECRET_ACCESS_KEY");
    }

    #[test]
    fn test_from_env_fallback_s3d_vars() {
        std::env::remove_var("CLOUDFLARE_R2_ACCESS_KEY_ID");
        std::env::remove_var("CLOUDFLARE_R2_SECRET_ACCESS_KEY");
        std::env::set_var("S3D_ACCESS_KEY_ID", "s3dkey");
        std::env::set_var("S3D_SECRET_ACCESS_KEY", "s3dsecret");

        let creds = StorageCredentials::from_env().unwrap();
        assert_eq!(creds.access_key_id, "s3dkey");

        std::env::remove_var("S3D_ACCESS_KEY_ID");
        std::env::remove_var("S3D_SECRET_ACCESS_KEY");
    }
}
