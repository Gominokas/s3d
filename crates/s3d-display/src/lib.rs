//! s3d-display — HTML 生成インターフェースと iframe 正規化
//!
//! ## 概要
//! `s3d-display` はフロント開発者向けの display 層クレートです。
//! `assetsStrategy` の宣言、`strategyAssets` の呼び出し、
//! iframe 正規化による HTML パーツ分割を提供します。
//!
//! ## モジュール構成
//!
//! | モジュール | 役割 |
//! |---|---|
//! | `config`   | `DisplayProjectConfig` の読み込み・バリデーション |
//! | `iframe`   | `<!-- s3d-part: id -->` マーカーによる HTML 分割 |
//! | `template` | 親ページ・パーツ HTML テンプレート生成 |
//! | `output`   | 出力ディレクトリへのファイル書き出し |
//! | `plugin`   | `PlainHtmlDisplay` — `DisplayPlugin` のデフォルト実装 |
//!
//! ## 使い方
//!
//! ```no_run
//! use s3d_display::{DisplayProjectConfig, PlainHtmlDisplay};
//! use s3d_types::plugin::{DisplayPlugin, RenderContext};
//!
//! // 設定を読み込む
//! let config = DisplayProjectConfig::from_file("s3d-display.json").unwrap();
//!
//! // プラグインを作成
//! let plugin = PlainHtmlDisplay::new(config);
//!
//! // HTML 生成（RenderContext は s3d-types で定義）
//! // let outputs = plugin.render(&context);
//! // s3d_display::write_outputs("output/", &outputs).unwrap();
//! ```

pub mod config;
pub mod iframe;
pub mod output;
pub mod plugin;
pub mod template;

// 主要な型を再エクスポート
pub use config::{ConfigError, DisplayProjectConfig, IframeConfig, IframePartRule};
pub use iframe::{partition_page, replace_iframe_markers, IframeMarker, IframePartition, Part};
pub use output::{collect_output_files, write_output_files, OutputError, OutputFile};
pub use plugin::PlainHtmlDisplay;
pub use template::{render_parent_page, render_part_page, TemplateOptions};

use std::path::Path;

use s3d_types::plugin::HtmlOutput;

// ─────────────────────────────────────────────
// ビルドエントリポイント
// ─────────────────────────────────────────────

/// `DisplayPlugin::render()` の結果を出力ディレクトリに書き出す便利関数
///
/// `HtmlOutput` の `path` は `output_dir` からの相対パスとして扱う。
pub fn write_outputs(output_dir: &str, outputs: &[HtmlOutput]) -> Result<(), OutputError> {
    let dir = Path::new(output_dir);
    let files: Vec<OutputFile> = outputs
        .iter()
        .map(|o| OutputFile {
            relative_path: o.path.clone(),
            content: o.content.clone(),
            cache_control: o.cache_control.clone(),
        })
        .collect();
    write_output_files(dir, &files).map(|_| ())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use s3d_types::config::{DisplayConfig, LoaderDisplayConfig, S3dConfig};
    use s3d_types::manifest::{AssetEntry, DeployManifest};
    use s3d_types::plugin::{DisplayPlugin, RenderContext};

    fn sample_manifest() -> DeployManifest {
        let mut assets = HashMap::new();
        assets.insert(
            "js/app.js".to_string(),
            AssetEntry {
                url: "https://cdn.example.com/js/app.abc.js".to_string(),
                size: 512,
                hash: "abc123".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        DeployManifest {
            schema_version: 1,
            version: "2.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
        }
    }

    fn sample_s3d_config() -> S3dConfig {
        S3dConfig {
            schema_version: 1,
            project: "s3d-test".to_string(),
            deploy: None,
            display: Some(DisplayConfig {
                loader: LoaderDisplayConfig {
                    concurrency: None,
                    retry_count: None,
                    retry_base_delay: None,
                    timeout: None,
                },
            }),
            draft: None,
        }
    }

    #[test]
    fn write_outputs_creates_files() {
        let tmp = TempDir::new().unwrap();
        let outputs = vec![
            HtmlOutput {
                path: "index.html".to_string(),
                content: "<html>parent</html>".to_string(),
                cache_control: None,
            },
            HtmlOutput {
                path: "parts/header.html".to_string(),
                content: "<html>header</html>".to_string(),
                cache_control: Some("max-age=3600".to_string()),
            },
        ];

        write_outputs(tmp.path().to_str().unwrap(), &outputs).unwrap();
        assert!(tmp.path().join("index.html").exists());
        assert!(tmp.path().join("parts/header.html").exists());
    }

    #[test]
    fn end_to_end_plain_html_display() {
        let config_json = serde_json::json!({
            "outputDir": "output",
            "manifestUrl": "https://cdn.example.com/manifest.json",
            "assetsStrategy": {
                "initial": {
                    "sources": ["js/main.js"],
                    "cache": true
                },
                "cdn": {
                    "files": ["models/**"],
                    "cache": true
                },
                "reload": {
                    "trigger": "manifest-change",
                    "strategy": "diff"
                }
            },
            "title": "E2E Test App"
        })
        .to_string();

        let cfg = DisplayProjectConfig::from_json(&config_json, "test.json").unwrap();
        let plugin = PlainHtmlDisplay::new(cfg);
        let s3d_config = sample_s3d_config();
        let manifest = sample_manifest();
        let context = RenderContext {
            config: &s3d_config,
            manifest: &manifest,
        };

        let outputs = plugin.render(&context);
        assert!(!outputs.is_empty());

        let index = &outputs[0];
        assert_eq!(index.path, "index.html");
        assert!(index.content.contains("E2E Test App"));
        assert!(index.content.contains("manifest-change"));
        assert!(index.content.contains("js/app.js")); // manifest キーがボディに含まれる
    }
}
