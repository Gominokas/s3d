//! 認証情報ヘルパー
//!
//! 環境変数 `S3D_ACCESS_KEY_ID` / `S3D_SECRET_ACCESS_KEY` を読み取る。

use anyhow::{Context, Result};

/// R2/S3 の認証情報
#[derive(Debug, Clone)]
pub struct StorageCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl StorageCredentials {
    /// 環境変数から認証情報を読み込む
    pub fn from_env() -> Result<Self> {
        let access_key_id = std::env::var("S3D_ACCESS_KEY_ID")
            .context("環境変数 S3D_ACCESS_KEY_ID が未設定です")?;
        let secret_access_key = std::env::var("S3D_SECRET_ACCESS_KEY")
            .context("環境変数 S3D_SECRET_ACCESS_KEY が未設定です")?;
        Ok(Self {
            access_key_id,
            secret_access_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_missing() {
        // Ensure vars are unset for this test
        std::env::remove_var("S3D_ACCESS_KEY_ID");
        std::env::remove_var("S3D_SECRET_ACCESS_KEY");
        assert!(StorageCredentials::from_env().is_err());
    }
}
