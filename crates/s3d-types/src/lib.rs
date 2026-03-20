//! s3d-types — s3d プロジェクト共通の型定義クレート
//!
//! TypeScript パッケージ `bk/static3d/packages/types/` に対応する Rust 型を提供する。

pub mod asset;
pub mod config;
pub mod loader;
pub mod manifest;
pub mod plugin;

// よく使う型を crate のトップレベルから参照できるよう re-export する
pub use asset::{AssetDiff, AssetStrategy, CollectedAsset, HashedAsset};
pub use config::{
    AssetsDeployConfig, CdnConfig, CdnProvider, DeployConfig, DisplayConfig, DraftConfig,
    DraftPreviewConfig, LoaderDisplayConfig, PagesConfig, S3dConfig,
};
pub use loader::{
    AssetResult, LoadAllOptions, LoadError, LoadErrorKind, LoadOptions, LoaderOptions,
    ProgressEvent, ResponseType,
};
pub use manifest::{AssetEntry, DeployManifest, StrategyEntry, StrategyReload};
pub use plugin::{
    DisplayPlugin, HtmlOutput, RenderContext, StorageError, StoragePlugin, StorageResult,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// `static3d.config.json` と同等の JSON を S3dConfig にデシリアライズし、
    /// 再シリアライズしても同じ値が得られることを確認する。
    #[test]
    fn config_roundtrip() {
        let json = r#"
        {
            "schemaVersion": 1,
            "project": "my-project",
            "deploy": {
                "pages": {
                    "outputDir": "dist",
                    "customDomain": "example.com"
                },
                "cdn": {
                    "provider": "cloudflare-r2",
                    "bucket": "my-bucket",
                    "baseUrl": "https://cdn.example.com",
                    "region": "auto"
                },
                "assets": {
                    "immediateDir": "assets/immediate",
                    "deferredDir": "assets/deferred",
                    "hashLength": 8,
                    "maxFileSize": "10MB",
                    "ignore": ["**/*.map"],
                    "include": ["**/*.js", "**/*.css"]
                },
                "oldVersionRetention": 3,
                "oldVersionMaxAge": "30d"
            },
            "display": {
                "loader": {
                    "concurrency": 4,
                    "retryCount": 3,
                    "retryBaseDelay": 500,
                    "timeout": 30000
                }
            },
            "draft": {
                "preview": {
                    "expiresIn": "1h"
                }
            }
        }
        "#;

        let config: S3dConfig = serde_json::from_str(json).expect("deserialization failed");

        assert_eq!(config.schema_version, 1);
        assert_eq!(config.project, "my-project");

        let deploy = config.deploy.as_ref().expect("deploy should be present");
        assert_eq!(deploy.pages.output_dir, "dist");
        assert_eq!(deploy.pages.custom_domain.as_deref(), Some("example.com"));
        assert_eq!(deploy.cdn.provider, CdnProvider::CloudflareR2);
        assert_eq!(deploy.cdn.bucket, "my-bucket");
        assert_eq!(deploy.cdn.base_url, "https://cdn.example.com");
        assert_eq!(deploy.cdn.region.as_deref(), Some("auto"));
        assert_eq!(deploy.assets.immediate_dir, "assets/immediate");
        assert_eq!(deploy.assets.deferred_dir, "assets/deferred");
        assert_eq!(deploy.assets.hash_length, Some(8));
        assert_eq!(deploy.old_version_retention, Some(3));
        assert_eq!(deploy.old_version_max_age.as_deref(), Some("30d"));

        let display = config.display.as_ref().expect("display should be present");
        assert_eq!(display.loader.concurrency, Some(4));
        assert_eq!(display.loader.retry_count, Some(3));
        assert_eq!(display.loader.retry_base_delay, Some(500));
        assert_eq!(display.loader.timeout, Some(30000));

        let draft = config.draft.as_ref().expect("draft should be present");
        assert_eq!(draft.preview.expires_in.as_deref(), Some("1h"));

        // re-serialize して再度パースできることを確認
        let serialized = serde_json::to_string(&config).expect("serialization failed");
        let config2: S3dConfig =
            serde_json::from_str(&serialized).expect("re-deserialization failed");
        assert_eq!(config2.project, config.project);
    }

    /// 最小構成（必須フィールドのみ）の S3dConfig を検証する。
    #[test]
    fn config_minimal() {
        let json = r#"{"schemaVersion": 1, "project": "minimal"}"#;
        let config: S3dConfig = serde_json::from_str(json).expect("deserialization failed");
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.project, "minimal");
        assert!(config.deploy.is_none());
        assert!(config.display.is_none());
        assert!(config.draft.is_none());
    }

    /// DeployManifest の JSON roundtrip を検証する。
    #[test]
    fn manifest_roundtrip() {
        use std::collections::HashMap;

        let mut assets = HashMap::new();
        assets.insert(
            "js/main.abc12345.js".to_string(),
            AssetEntry {
                url: "https://cdn.example.com/js/main.abc12345.js".to_string(),
                size: 102400,
                hash: "abc12345def67890".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: Some(vec!["js/vendor.xyz.js".to_string()]),
            },
        );

        let manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
            strategies: std::collections::HashMap::new(),
        };

        let json = serde_json::to_string(&manifest).expect("serialization failed");
        let manifest2: DeployManifest =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(manifest2.schema_version, manifest.schema_version);
        assert_eq!(manifest2.version, manifest.version);
        assert!(manifest2.assets.contains_key("js/main.abc12345.js"));
    }

    /// LoadError の kind フィールドが kebab-case でシリアライズされることを確認する。
    #[test]
    fn load_error_kind_serialization() {
        let err = LoadError {
            kind: LoadErrorKind::NotFound,
            key: "js/main.js".to_string(),
            url: "https://cdn.example.com/js/main.js".to_string(),
            cause: None,
            status_code: Some(404),
        };
        let json = serde_json::to_string(&err).expect("serialization failed");
        assert!(json.contains("not-found"), "expected kebab-case: {json}");
    }

    /// AssetDiff と AssetStrategy が lowercase でシリアライズされることを確認する。
    #[test]
    fn asset_enum_serialization() {
        let diff = AssetDiff::Modified;
        let s = serde_json::to_string(&diff).expect("failed");
        assert_eq!(s, "\"modified\"");

        let strategy = AssetStrategy::Cdn;
        let s = serde_json::to_string(&strategy).expect("failed");
        assert_eq!(s, "\"cdn\"");
    }

    /// StoragePlugin が async fn を持ち、dyn として扱えることを確認する。
    #[tokio::test]
    async fn storage_plugin_is_async() {
        use async_trait::async_trait;

        struct MockStorage;

        #[async_trait]
        impl StoragePlugin for MockStorage {
            async fn put(&self, _key: &str, _data: &[u8], _ct: &str) -> StorageResult<()> {
                Ok(())
            }
            async fn get(&self, key: &str) -> StorageResult<Vec<u8>> {
                Ok(key.as_bytes().to_vec())
            }
            async fn delete(&self, _key: &str) -> StorageResult<()> {
                Ok(())
            }
            async fn list(&self, _prefix: &str) -> StorageResult<Vec<String>> {
                Ok(vec!["a".to_string(), "b".to_string()])
            }
        }

        // dyn StoragePlugin として Box に詰めて呼び出せることを確認
        let storage: Box<dyn StoragePlugin> = Box::new(MockStorage);

        assert!(storage.put("k", b"v", "text/plain").await.is_ok());

        let data = storage.get("hello").await.unwrap();
        assert_eq!(data, b"hello");

        assert!(storage.delete("k").await.is_ok());

        let keys = storage.list("").await.unwrap();
        assert_eq!(keys, vec!["a", "b"]);
    }
}
