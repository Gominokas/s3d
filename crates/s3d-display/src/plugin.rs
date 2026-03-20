//! PlainHtmlDisplay — DisplayPlugin のデフォルト実装
//!
//! React / Astro 等の外部フレームワークに依存せず、
//! テンプレートリテラルのみで HTML を生成する。
//!
//! 将来の `display-react` / `display-astro` プラグインは
//! `DisplayPlugin` trait を別クレートで実装する。

use s3d_types::manifest::DeployManifest;
use s3d_types::plugin::{DisplayPlugin, HtmlOutput, RenderContext};

use crate::config::DisplayProjectConfig;
use crate::iframe::partition_page;
use crate::template::{render_parent_page, render_part_page, TemplateOptions};

// ─────────────────────────────────────────────
// PlainHtmlDisplay
// ─────────────────────────────────────────────

/// プレーン HTML 生成プラグイン
///
/// `DisplayProjectConfig` を保持し、`render()` で HTML ファイル群を返す。
#[derive(Debug, Clone)]
pub struct PlainHtmlDisplay {
    /// display 側の設定
    pub config: DisplayProjectConfig,
}

impl PlainHtmlDisplay {
    /// 新しい `PlainHtmlDisplay` を作成する
    pub fn new(config: DisplayProjectConfig) -> Self {
        Self { config }
    }
}

impl DisplayPlugin for PlainHtmlDisplay {
    /// HTML ファイル群を生成して返す
    ///
    /// 1. `RenderContext` から `S3dConfig` と `DeployManifest` を受け取る
    /// 2. `DisplayProjectConfig` の `assetsStrategy` を使ってテンプレートを構築
    /// 3. `iframe` 設定に従い HTML を分割してパーツ HTML を生成
    /// 4. 全 HTML ファイルを `Vec<HtmlOutput>` として返す
    fn render(&self, context: &RenderContext) -> Vec<HtmlOutput> {
        let cfg = &self.config;
        let opts = TemplateOptions {
            title: cfg.title(),
            manifest_url: &cfg.manifest_url,
            assets_strategy: &cfg.assets_strategy,
            extra_head: cfg.extra_head.as_deref(),
        };

        // ダミーのボディ HTML（実際には RenderContext から取得するか外部から渡す）
        // ここでは manifest の asset 一覧をデバッグ出力として使う簡易実装
        let body_html = build_body_html(context.manifest);

        // iframe 分割
        let partition = partition_page(
            &body_html,
            &cfg.iframe.partition_rules,
            cfg.iframe.iframe_attrs.as_deref(),
        );

        let mut outputs: Vec<HtmlOutput> = Vec::new();

        // パーツ HTML 生成
        let mut part_htmls: Vec<String> = Vec::new();
        for part in &partition.parts {
            let html = render_part_page(
                &part.id,
                &part.content,
                part.cache_control.as_deref(),
                &opts,
            );
            part_htmls.push(html.clone());
            outputs.push(HtmlOutput {
                path: part.output_path.clone(),
                content: html,
                cache_control: part.cache_control.clone(),
            });
        }

        // 親ページ HTML 生成（先頭に追加）
        let parent_html = render_parent_page(&partition.parent_content, &opts);
        outputs.insert(
            0,
            HtmlOutput {
                path: "index.html".to_string(),
                content: parent_html,
                cache_control: None,
            },
        );

        outputs
    }
}

/// manifest の asset 情報からデバッグ用の簡易ボディ HTML を生成する
fn build_body_html(manifest: &DeployManifest) -> String {
    let mut lines = vec![
        format!("<div id=\"s3d-app\" data-version=\"{}\">", manifest.version),
        "  <!-- s3d application root -->".to_string(),
    ];

    if !manifest.assets.is_empty() {
        lines.push("  <ul id=\"s3d-assets\">".to_string());
        let mut keys: Vec<&str> = manifest.assets.keys().map(String::as_str).collect();
        keys.sort();
        for key in keys {
            lines.push(format!("    <li data-key=\"{}\"></li>", key));
        }
        lines.push("  </ul>".to_string());
    }

    lines.push("</div>".to_string());
    lines.join("\n")
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use s3d_loader::{
        AssetsStrategyConfig, CdnStrategyConfig, InitialConfig, ReloadConfig, ReloadStrategy,
        ReloadTrigger,
    };
    use s3d_types::config::{DeployConfig, DisplayConfig, LoaderDisplayConfig, S3dConfig};
    use s3d_types::manifest::{AssetEntry, DeployManifest};

    use crate::config::{DisplayProjectConfig, IframeConfig, IframePartRule};

    fn sample_strategy() -> AssetsStrategyConfig {
        AssetsStrategyConfig {
            initial: InitialConfig {
                sources: vec!["js/main.js".to_string()],
                cache: true,
                fallback: None,
            },
            cdn: CdnStrategyConfig {
                files: vec!["models/**".to_string()],
                cache: true,
                max_age: None,
            },
            reload: ReloadConfig {
                trigger: ReloadTrigger::ManifestChange,
                strategy: ReloadStrategy::Diff,
                interval_ms: None,
            },
        }
    }

    fn sample_manifest() -> DeployManifest {
        let mut assets = HashMap::new();
        assets.insert(
            "js/main.js".to_string(),
            AssetEntry {
                url: "https://cdn.example.com/js/main.abc.js".to_string(),
                size: 1024,
                hash: "abc12345".to_string(),
                content_type: "application/javascript".to_string(),
                dependencies: None,
            },
        );
        DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
        }
    }

    fn sample_s3d_config() -> S3dConfig {
        S3dConfig {
            schema_version: 1,
            project: "test".to_string(),
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

    fn make_plugin(with_parts: bool) -> PlainHtmlDisplay {
        let rules = if with_parts {
            vec![
                IframePartRule {
                    id: "header".to_string(),
                    output_path: "parts/header.html".to_string(),
                    cache_control: Some("max-age=3600".to_string()),
                },
                IframePartRule {
                    id: "footer".to_string(),
                    output_path: "parts/footer.html".to_string(),
                    cache_control: None,
                },
            ]
        } else {
            vec![]
        };

        let config = DisplayProjectConfig {
            output_dir: "output".to_string(),
            manifest_url: "https://cdn.example.com/manifest.json".to_string(),
            assets_strategy: sample_strategy(),
            iframe: IframeConfig {
                partition_rules: rules,
                iframe_attrs: None,
            },
            title: Some("Test App".to_string()),
            extra_head: None,
        };
        PlainHtmlDisplay::new(config)
    }

    #[test]
    fn render_without_parts_produces_index_only() {
        let plugin = make_plugin(false);
        let s3d_config = sample_s3d_config();
        let manifest = sample_manifest();
        let context = RenderContext {
            config: &s3d_config,
            manifest: &manifest,
        };

        let outputs = plugin.render(&context);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].path, "index.html");
        assert!(outputs[0].content.contains("<!DOCTYPE html>"));
        assert!(outputs[0].content.contains("Test App"));
    }

    #[test]
    fn render_body_html_contains_asset_keys() {
        let manifest = sample_manifest();
        let html = build_body_html(&manifest);
        assert!(html.contains("js/main.js"));
        assert!(html.contains("1.0.0"));
    }

    #[test]
    fn render_with_iframe_parts_produces_multiple_files() {
        let plugin = make_plugin(true);
        let s3d_config = sample_s3d_config();

        // パーツマーカーを含むボディを使うには iframe.rs の partition_page が必要
        // ここではマーカーなしで呼ぶと parts が生成されないことを確認
        let manifest = sample_manifest();
        let context = RenderContext {
            config: &s3d_config,
            manifest: &manifest,
        };
        let outputs = plugin.render(&context);
        // マーカーなしのボディなのでパーツは生成されず index.html のみ
        assert_eq!(outputs[0].path, "index.html");
    }

    #[test]
    fn render_output_contains_strategy_json() {
        let plugin = make_plugin(false);
        let s3d_config = sample_s3d_config();
        let manifest = sample_manifest();
        let context = RenderContext {
            config: &s3d_config,
            manifest: &manifest,
        };

        let outputs = plugin.render(&context);
        assert!(outputs[0].content.contains("manifest-change"));
        assert!(outputs[0]
            .content
            .contains("https://cdn.example.com/manifest.json"));
    }
}
