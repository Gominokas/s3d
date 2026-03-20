//! display 側の設定読み込み・バリデーションモジュール
//!
//! `DisplayProjectConfig` は `s3d-display.json` または
//! `s3d.config.json` の `display` セクションから読み込む。
//! `s3d-loader` の `AssetsStrategyConfig` を再利用し、
//! iframe 分割ルールも同じファイルに宣言する。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use s3d_loader::AssetsStrategyConfig;

// ─────────────────────────────────────────────
// エラー型
// ─────────────────────────────────────────────

/// config モジュールのエラー型
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error reading config `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON parse error in `{path}`: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Validation error: {0}")]
    Validation(String),
}

// ─────────────────────────────────────────────
// IframePartRule — 分割ルール1件
// ─────────────────────────────────────────────

/// 1パーツの分割ルール
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IframePartRule {
    /// パーツ ID（マーカーコメント `<!-- s3d-part: <id> -->` と対応）
    pub id: String,
    /// 出力先相対パス（例: `"parts/header.html"`）
    pub output_path: String,
    /// Cache-Control ヘッダー値（省略可）
    pub cache_control: Option<String>,
}

// ─────────────────────────────────────────────
// IframeConfig — iframe 正規化の設定
// ─────────────────────────────────────────────

/// iframe 正規化の設定
///
/// `partition_rules` が空の場合、分割は行わずページ全体を単一 HTML として出力する。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IframeConfig {
    /// 分割ルール一覧
    pub partition_rules: Vec<IframePartRule>,
    /// 親ページでのデフォルト iframe 属性（例: `"width=100% frameborder=0"`）
    pub iframe_attrs: Option<String>,
}

// ─────────────────────────────────────────────
// DisplayProjectConfig — トップレベル設定
// ─────────────────────────────────────────────

/// s3d-display プロジェクト設定
///
/// `s3d-display.json` または `s3d.config.json` の `display` セクションに対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayProjectConfig {
    /// HTML 出力先ディレクトリ（例: `"output"`）
    pub output_dir: String,
    /// CDN manifest の URL（例: `"https://cdn.example.com/manifest.json"`）
    pub manifest_url: String,
    /// アセット配信戦略（s3d-loader の AssetsStrategyConfig を再利用）
    pub assets_strategy: AssetsStrategyConfig,
    /// iframe 分割設定
    #[serde(default)]
    pub iframe: IframeConfig,
    /// ページタイトル（省略時は `"s3d app"`）
    pub title: Option<String>,
    /// 追加で `<head>` に挿入する HTML スニペット（省略可）
    pub extra_head: Option<String>,
}

// ─────────────────────────────────────────────
// 読み込み・バリデーション
// ─────────────────────────────────────────────

impl DisplayProjectConfig {
    /// JSON ファイルから設定を読み込む
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_string(),
            source,
        })?;
        Self::from_json(&content, path)
    }

    /// JSON 文字列から設定をパースする
    pub fn from_json(json: &str, source_path: &str) -> Result<Self, ConfigError> {
        let cfg: Self = serde_json::from_str(json).map_err(|source| ConfigError::Parse {
            path: source_path.to_string(),
            source,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// バリデーション
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.output_dir.is_empty() {
            return Err(ConfigError::Validation(
                "outputDir must not be empty".to_string(),
            ));
        }
        if self.manifest_url.is_empty() {
            return Err(ConfigError::Validation(
                "manifestUrl must not be empty".to_string(),
            ));
        }
        // partition_rules の id 重複チェック
        let mut ids = std::collections::HashSet::new();
        for rule in &self.iframe.partition_rules {
            if rule.id.is_empty() {
                return Err(ConfigError::Validation(
                    "partition rule id must not be empty".to_string(),
                ));
            }
            if !ids.insert(rule.id.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate partition rule id: `{}`",
                    rule.id
                )));
            }
        }
        Ok(())
    }

    /// ページタイトルを返す（デフォルト: `"s3d app"`）
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("s3d app")
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use s3d_loader::{
        CdnStrategyConfig, InitialConfig, ReloadConfig, ReloadStrategy, ReloadTrigger,
    };

    fn sample_json() -> String {
        serde_json::json!({
            "outputDir": "output",
            "manifestUrl": "https://cdn.example.com/manifest.json",
            "assetsStrategy": {
                "initial": {
                    "sources": ["js/main.js", "style.css"],
                    "cache": true
                },
                "cdn": {
                    "files": ["models/**/*.glb"],
                    "cache": true
                },
                "reload": {
                    "trigger": "manifest-change",
                    "strategy": "diff"
                }
            },
            "iframe": {
                "partitionRules": [
                    { "id": "header", "outputPath": "parts/header.html", "cacheControl": "max-age=3600" },
                    { "id": "main",   "outputPath": "parts/main.html" },
                    { "id": "footer", "outputPath": "parts/footer.html" }
                ]
            },
            "title": "My 3D App"
        })
        .to_string()
    }

    #[test]
    fn parse_full_config() {
        let cfg = DisplayProjectConfig::from_json(&sample_json(), "test.json").unwrap();
        assert_eq!(cfg.output_dir, "output");
        assert_eq!(cfg.manifest_url, "https://cdn.example.com/manifest.json");
        assert_eq!(cfg.title(), "My 3D App");
        assert_eq!(cfg.iframe.partition_rules.len(), 3);
        assert_eq!(cfg.iframe.partition_rules[0].id, "header");
        assert_eq!(
            cfg.iframe.partition_rules[0].cache_control,
            Some("max-age=3600".to_string())
        );
        assert!(cfg.iframe.partition_rules[1].cache_control.is_none());
    }

    #[test]
    fn default_title() {
        let mut json: serde_json::Value = serde_json::from_str(&sample_json()).unwrap();
        json.as_object_mut().unwrap().remove("title");
        let cfg = DisplayProjectConfig::from_json(&json.to_string(), "test.json").unwrap();
        assert_eq!(cfg.title(), "s3d app");
    }

    #[test]
    fn validation_empty_output_dir() {
        let mut json: serde_json::Value = serde_json::from_str(&sample_json()).unwrap();
        json["outputDir"] = serde_json::json!("");
        let result = DisplayProjectConfig::from_json(&json.to_string(), "test.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("outputDir"));
    }

    #[test]
    fn validation_duplicate_part_id() {
        let mut json: serde_json::Value = serde_json::from_str(&sample_json()).unwrap();
        json["iframe"]["partitionRules"] = serde_json::json!([
            { "id": "header", "outputPath": "parts/header.html" },
            { "id": "header", "outputPath": "parts/header2.html" }
        ]);
        let result = DisplayProjectConfig::from_json(&json.to_string(), "test.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn assets_strategy_roundtrip() {
        let cfg = DisplayProjectConfig::from_json(&sample_json(), "test.json").unwrap();
        assert_eq!(
            cfg.assets_strategy.initial.sources,
            vec!["js/main.js", "style.css"]
        );
        assert_eq!(
            cfg.assets_strategy.reload.trigger,
            ReloadTrigger::ManifestChange
        );
        assert_eq!(cfg.assets_strategy.reload.strategy, ReloadStrategy::Diff);
    }
}
