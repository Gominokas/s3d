//! HTML テンプレート生成モジュール
//!
//! 親ページ・パーツ HTML のテンプレートを生成する。
//! `strategyAssets` の初期化スクリプトタグを埋め込み、
//! Cache-Control を `<meta>` タグで表現する。
//!
//! ## 出力例（親ページ）
//!
//! ```html
//! <!DOCTYPE html>
//! <html lang="ja">
//! <head>
//!   <meta charset="UTF-8">
//!   <title>My App</title>
//!   <script type="application/json" id="s3d-strategy">{ ... }</script>
//!   <script src="assets/s3d-loader.js" defer></script>
//! </head>
//! <body>
//!   <!-- parent_body -->
//! </body>
//! </html>
//! ```

use s3d_loader::AssetsStrategyConfig;

// ─────────────────────────────────────────────
// TemplateOptions
// ─────────────────────────────────────────────

/// テンプレート生成オプション
#[derive(Debug, Clone)]
pub struct TemplateOptions<'a> {
    /// ページタイトル
    pub title: &'a str,
    /// manifest URL
    pub manifest_url: &'a str,
    /// アセット配信戦略
    pub assets_strategy: &'a AssetsStrategyConfig,
    /// `<head>` に追加する HTML スニペット（省略可）
    pub extra_head: Option<&'a str>,
}

// ─────────────────────────────────────────────
// 親ページ HTML 生成
// ─────────────────────────────────────────────

/// 親ページの完全な HTML を生成する
///
/// `body_content` にはすでに `<iframe>` タグを含むパーツ分割済みの HTML が渡される。
pub fn render_parent_page(body_content: &str, opts: &TemplateOptions<'_>) -> String {
    let strategy_json =
        serde_json::to_string_pretty(opts.assets_strategy).unwrap_or_else(|_| "{}".to_string());

    let extra = opts.extra_head.unwrap_or("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title}</title>
  <script type="application/json" id="s3d-strategy">
{strategy_json}
  </script>
  <script type="text/javascript">
    (function() {{
      var cfg = JSON.parse(document.getElementById('s3d-strategy').textContent);
      window.__S3D_CONFIG__ = {{
        manifestUrl: {manifest_url_json},
        strategy: cfg
      }};
    }})();
  </script>{extra_head}
</head>
<body>
{body_content}
</body>
</html>"#,
        title = escape_html(opts.title),
        strategy_json = strategy_json,
        manifest_url_json = serde_json::to_string(opts.manifest_url).unwrap_or_default(),
        extra_head = if extra.is_empty() {
            String::new()
        } else {
            format!("\n  {}", extra)
        },
        body_content = body_content,
    )
}

// ─────────────────────────────────────────────
// パーツ HTML 生成
// ─────────────────────────────────────────────

/// パーツの完全な HTML を生成する
///
/// 各パーツは独立した HTML ファイルとして出力され、
/// `cache_control` が指定されていれば `<meta>` タグとして埋め込む。
pub fn render_part_page(
    part_id: &str,
    body_content: &str,
    cache_control: Option<&str>,
    opts: &TemplateOptions<'_>,
) -> String {
    let cc_meta = cache_control
        .map(|cc| {
            format!(
                "\n  <meta http-equiv=\"Cache-Control\" content=\"{}\">",
                escape_html(cc)
            )
        })
        .unwrap_or_default();

    let strategy_json =
        serde_json::to_string_pretty(opts.assets_strategy).unwrap_or_else(|_| "{}".to_string());

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title} — {part_id}</title>{cc_meta}
  <script type="application/json" id="s3d-strategy">
{strategy_json}
  </script>
  <script type="text/javascript">
    (function() {{
      var cfg = JSON.parse(document.getElementById('s3d-strategy').textContent);
      window.__S3D_CONFIG__ = {{
        manifestUrl: {manifest_url_json},
        strategy: cfg,
        partId: {part_id_json}
      }};
    }})();
  </script>
</head>
<body>
{body_content}
</body>
</html>"#,
        title = escape_html(opts.title),
        part_id = escape_html(part_id),
        cc_meta = cc_meta,
        strategy_json = strategy_json,
        manifest_url_json = serde_json::to_string(opts.manifest_url).unwrap_or_default(),
        part_id_json = serde_json::to_string(part_id).unwrap_or_default(),
        body_content = body_content,
    )
}

// ─────────────────────────────────────────────
// ヘルパー
// ─────────────────────────────────────────────

/// HTML 特殊文字をエスケープする（タイトル等に使用）
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use s3d_loader::{
        AssetsStrategyConfig, CdnStrategyConfig, InitialConfig, ReloadConfig, ReloadStrategy,
        ReloadTrigger,
    };

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

    fn sample_opts(strategy: &AssetsStrategyConfig) -> TemplateOptions<'_> {
        TemplateOptions {
            title: "Test App",
            manifest_url: "https://cdn.example.com/manifest.json",
            assets_strategy: strategy,
            extra_head: None,
        }
    }

    #[test]
    fn parent_page_contains_doctype() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_parent_page("<p>body</p>", &opts);
        assert!(html.starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn parent_page_contains_title() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_parent_page("", &opts);
        assert!(html.contains("<title>Test App</title>"));
    }

    #[test]
    fn parent_page_contains_strategy_json() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_parent_page("", &opts);
        assert!(html.contains(r#"id="s3d-strategy""#));
        assert!(html.contains("manifest-change"));
    }

    #[test]
    fn parent_page_contains_manifest_url() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_parent_page("", &opts);
        assert!(html.contains("https://cdn.example.com/manifest.json"));
    }

    #[test]
    fn parent_page_extra_head_inserted() {
        let strategy = sample_strategy();
        let mut opts = sample_opts(&strategy);
        opts.extra_head = Some(r#"<link rel="stylesheet" href="style.css">"#);
        let html = render_parent_page("", &opts);
        assert!(html.contains(r#"<link rel="stylesheet""#));
    }

    #[test]
    fn part_page_contains_cache_control_meta() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_part_page("header", "<h1>Hi</h1>", Some("max-age=3600"), &opts);
        assert!(html.contains("Cache-Control"));
        assert!(html.contains("max-age=3600"));
    }

    #[test]
    fn part_page_no_cache_control_when_none() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_part_page("main", "<p>Content</p>", None, &opts);
        assert!(!html.contains("Cache-Control"));
    }

    #[test]
    fn part_page_contains_part_id_in_config() {
        let strategy = sample_strategy();
        let opts = sample_opts(&strategy);
        let html = render_part_page("footer", "<footer>F</footer>", None, &opts);
        assert!(html.contains("\"footer\""));
    }

    #[test]
    fn html_title_is_escaped() {
        let strategy = sample_strategy();
        let mut opts = sample_opts(&strategy);
        opts.title = "<script>alert(1)</script>";
        let html = render_parent_page("", &opts);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }
}
