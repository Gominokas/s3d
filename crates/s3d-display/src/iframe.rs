//! iframe 正規化モジュール
//!
//! 2種類のマーカー変換をサポートする:
//!
//! ## 1. `s3d-part` — コンテンツ分割型
//!
//! HTML を `<!-- s3d-part: <id> -->` コメントマーカーでパーツに分割し、
//! 各パーツを独立 HTML ファイルとして出力する。
//! 親ページは `<iframe src="parts/<id>.html">` でパーツを参照する。
//!
//! ```text
//! <!-- s3d-part: header -->
//! <header>...</header>
//! <!-- s3d-part-end -->
//! ```
//!
//! ## 2. `s3d-iframe` — 動的データ領域の切り出し型 (Issue #10)
//!
//! SSGページ内の動的データ領域（価格、在庫数、ステータス等）をiframeとして切り出す。
//! データ変更時に親ページの再ビルドを不要にし、差分取得対象にできる。
//!
//! ```text
//! <!-- s3d-iframe: price-123 src="parts/price-123.html" -->
//! ```
//!
//! - マーカーは単体タグ（開始/終了なし）— そのまま `<iframe>` タグに変換
//! - 参照先 HTML は別途生成・配置する（このマーカーはリンクのみ）
//! - `data-s3d-id` 属性でクライアント側から識別可能
//! - 親ページは長期キャッシュ、iframe 参照先は短期キャッシュを推奨

use crate::config::IframePartRule;

// ─────────────────────────────────────────────
// Part — 分割されたパーツ1件
// ─────────────────────────────────────────────

/// HTML 分割後の1パーツ
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// パーツ ID（マーカーの `<id>` 部分）
    pub id: String,
    /// パーツ本体の HTML コンテンツ（`<body>` 内側のみ）
    pub content: String,
    /// 設定から解決した出力先パス（例: `"parts/header.html"`）
    pub output_path: String,
    /// Cache-Control ヘッダー値
    pub cache_control: Option<String>,
}

// ─────────────────────────────────────────────
// IframePartition — 分割結果
// ─────────────────────────────────────────────

/// `partition_page()` の返却値
#[derive(Debug, Clone)]
pub struct IframePartition {
    /// 元の HTML からパーツを除いた「親ページ本体」の内容
    /// （マーカーが `<iframe>` タグに置換済み）
    pub parent_content: String,
    /// 分割されたパーツ一覧
    pub parts: Vec<Part>,
}

// ─────────────────────────────────────────────
// partition_page
// ─────────────────────────────────────────────

/// HTML 文字列をパーツに分割する
///
/// - `html`: 元の HTML 文字列
/// - `rules`: `IframePartRule` のスライス（ID → output_path / cache_control のマッピング）
/// - `iframe_attrs`: `<iframe>` タグに追加する属性文字列（省略可）
///
/// マーカーが見つからない場合は `parts` が空の `IframePartition` を返す。
pub fn partition_page(
    html: &str,
    rules: &[IframePartRule],
    iframe_attrs: Option<&str>,
) -> IframePartition {
    let attrs = iframe_attrs.unwrap_or("width=\"100%\" frameborder=\"0\"");
    let mut parent = String::with_capacity(html.len());
    let mut parts: Vec<Part> = Vec::new();

    let mut remaining = html;

    while !remaining.is_empty() {
        // 開始マーカーを探す: `<!-- s3d-part: <id> -->`
        if let Some(start_pos) = find_part_start(remaining) {
            // マーカー前の部分を親ページに追加
            parent.push_str(&remaining[..start_pos.marker_start]);

            let part_id = start_pos.part_id.clone();

            // 終了マーカーを探す
            let after_start = &remaining[start_pos.marker_end..];
            if let Some(end_pos) = find_part_end(after_start) {
                let content = after_start[..end_pos.marker_start].trim().to_string();

                // output_path と cache_control をルールから解決
                let (output_path, cache_control) = resolve_rule(rules, &part_id);

                // `<iframe>` タグで置換（src は output_path の basename）
                let iframe_src = output_path.clone();
                parent.push_str(&format!(
                    "<iframe src=\"{}\" {}></iframe>",
                    iframe_src, attrs
                ));

                parts.push(Part {
                    id: part_id,
                    content,
                    output_path,
                    cache_control,
                });

                remaining = &remaining[start_pos.marker_end + end_pos.marker_end..];
            } else {
                // 終了マーカーなし → 残りをそのまま親ページに追加して終了
                parent.push_str(&remaining[start_pos.marker_start..]);
                remaining = "";
            }
        } else {
            // これ以上マーカーなし → 残りをすべて親ページに追加
            parent.push_str(remaining);
            remaining = "";
        }
    }

    IframePartition {
        parent_content: parent,
        parts,
    }
}

// ─────────────────────────────────────────────
// 内部ヘルパー
// ─────────────────────────────────────────────

struct MarkerPos {
    /// HTML 内でのマーカー開始位置
    marker_start: usize,
    /// マーカー終了位置（次の解析開始位置）
    marker_end: usize,
    /// パーツ ID
    part_id: String,
}

/// `<!-- s3d-part: <id> -->` を探して位置と ID を返す
fn find_part_start(html: &str) -> Option<MarkerPos> {
    const PREFIX: &str = "<!-- s3d-part:";
    const SUFFIX: &str = "-->";

    let start = html.find(PREFIX)?;
    let after_prefix = &html[start + PREFIX.len()..];
    let end_rel = after_prefix.find(SUFFIX)?;
    let part_id = after_prefix[..end_rel].trim().to_string();
    let marker_end = start + PREFIX.len() + end_rel + SUFFIX.len();

    Some(MarkerPos {
        marker_start: start,
        marker_end,
        part_id,
    })
}

/// `<!-- s3d-part-end -->` を探して位置を返す
fn find_part_end(html: &str) -> Option<MarkerPos> {
    const MARKER: &str = "<!-- s3d-part-end -->";
    let start = html.find(MARKER)?;
    Some(MarkerPos {
        marker_start: start,
        marker_end: start + MARKER.len(),
        part_id: String::new(),
    })
}

/// ルール一覧から ID に対応する `(output_path, cache_control)` を返す
/// ルールが見つからない場合はデフォルトのパスを生成する
fn resolve_rule(rules: &[IframePartRule], id: &str) -> (String, Option<String>) {
    if let Some(rule) = rules.iter().find(|r| r.id == id) {
        (rule.output_path.clone(), rule.cache_control.clone())
    } else {
        // ルールが未定義の場合はデフォルトパスを生成
        (format!("parts/{}.html", id), None)
    }
}

// ─────────────────────────────────────────────
// s3d-iframe マーカー変換 (Issue #10)
// ─────────────────────────────────────────────

/// `<!-- s3d-iframe: {id} src="{url}" -->` マーカーの変換結果
#[derive(Debug, Clone, PartialEq)]
pub struct IframeMarker {
    /// `data-s3d-id` に設定される識別子
    pub id: String,
    /// iframe の `src` 属性値
    pub src: String,
}

/// HTML 内の `<!-- s3d-iframe: {id} src="{url}" -->` を
/// `<iframe src="{url}" data-s3d-id="{id}" ...>` タグに変換する
///
/// ## 引数
/// - `html`: 変換対象の HTML 文字列
/// - `iframe_attrs`: `<iframe>` タグに追加する属性文字列（省略時はデフォルト属性）
///
/// ## 戻り値
/// `(変換後 HTML, 発見したマーカー一覧)`
///
/// マーカーが 1 件も見つからない場合は元の HTML をそのまま返す。
pub fn replace_iframe_markers(
    html: &str,
    iframe_attrs: Option<&str>,
) -> (String, Vec<IframeMarker>) {
    let attrs = iframe_attrs.unwrap_or("width=\"100%\" frameborder=\"0\"");
    let mut result = String::with_capacity(html.len());
    let mut markers: Vec<IframeMarker> = Vec::new();
    let mut remaining = html;

    while !remaining.is_empty() {
        if let Some(pos) = find_iframe_marker(remaining) {
            // マーカー前のテキストをそのまま追加
            result.push_str(&remaining[..pos.marker_start]);

            // `<iframe>` タグに変換
            result.push_str(&format!(
                "<iframe src=\"{}\" data-s3d-id=\"{}\" {}></iframe>",
                pos.src, pos.id, attrs
            ));

            markers.push(IframeMarker {
                id: pos.id,
                src: pos.src,
            });

            remaining = &remaining[pos.marker_end..];
        } else {
            result.push_str(remaining);
            remaining = "";
        }
    }

    (result, markers)
}

// ─────────────────────────────────────────────
// s3d-iframe 内部ヘルパー
// ─────────────────────────────────────────────

struct IframeMarkerPos {
    marker_start: usize,
    marker_end: usize,
    id: String,
    src: String,
}

/// `<!-- s3d-iframe: {id} src="{url}" -->` を探してパースする
///
/// 書式: `<!-- s3d-iframe: <id> src="<url>" -->`
/// - `<id>` は空白を含まないトークン
/// - `src="..."` は二重引用符で囲まれた URL
fn find_iframe_marker(html: &str) -> Option<IframeMarkerPos> {
    const PREFIX: &str = "<!-- s3d-iframe:";
    const SUFFIX: &str = "-->";

    let start = html.find(PREFIX)?;
    let after_prefix = &html[start + PREFIX.len()..];
    let end_rel = after_prefix.find(SUFFIX)?;
    let inner = after_prefix[..end_rel].trim();

    // inner を空白で分割: 先頭が id、残りに `src="..."` が含まれる
    // 例: `price-123 src="parts/price-123.html"`
    let (id, rest) = inner.split_once(char::is_whitespace)?;
    let id = id.trim().to_string();
    if id.is_empty() {
        return None;
    }

    // src="..." を抽出
    let src = parse_src_attr(rest.trim())?;

    let marker_end = start + PREFIX.len() + end_rel + SUFFIX.len();

    Some(IframeMarkerPos {
        marker_start: start,
        marker_end,
        id,
        src,
    })
}

/// `src="<url>"` または `src='<url>'` から URL 文字列を抽出する
fn parse_src_attr(s: &str) -> Option<String> {
    // `src=` を探す
    let src_pos = s.find("src=")?;
    let after_eq = &s[src_pos + 4..]; // "src=" の後
    let quote = after_eq.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let content = &after_eq[1..];
    let end = content.find(quote)?;
    Some(content[..end].to_string())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IframePartRule;

    fn rules() -> Vec<IframePartRule> {
        vec![
            IframePartRule {
                id: "header".to_string(),
                output_path: "parts/header.html".to_string(),
                cache_control: Some("max-age=3600".to_string()),
            },
            IframePartRule {
                id: "main".to_string(),
                output_path: "parts/main.html".to_string(),
                cache_control: None,
            },
            IframePartRule {
                id: "footer".to_string(),
                output_path: "parts/footer.html".to_string(),
                cache_control: Some("max-age=86400".to_string()),
            },
        ]
    }

    #[test]
    fn partition_three_parts() {
        let html = r#"<!DOCTYPE html>
<html>
<body>
<!-- s3d-part: header -->
<header><h1>Title</h1></header>
<!-- s3d-part-end -->
<main>
<!-- s3d-part: main -->
<p>Content</p>
<!-- s3d-part-end -->
</main>
<!-- s3d-part: footer -->
<footer>Footer</footer>
<!-- s3d-part-end -->
</body>
</html>"#;

        let result = partition_page(html, &rules(), None);
        assert_eq!(result.parts.len(), 3);

        let header = result.parts.iter().find(|p| p.id == "header").unwrap();
        assert!(header.content.contains("<header>"));
        assert_eq!(header.output_path, "parts/header.html");
        assert_eq!(header.cache_control, Some("max-age=3600".to_string()));

        let main = result.parts.iter().find(|p| p.id == "main").unwrap();
        assert!(main.content.contains("<p>Content</p>"));
        assert!(main.cache_control.is_none());

        // 親ページには <iframe> タグが含まれる
        assert!(result.parent_content.contains("<iframe"));
        assert!(result.parent_content.contains("parts/header.html"));
        assert!(result.parent_content.contains("parts/main.html"));
        assert!(result.parent_content.contains("parts/footer.html"));
        // 元のパーツコンテンツは残っていない
        assert!(!result.parent_content.contains("<header>"));
    }

    #[test]
    fn no_markers_returns_full_html() {
        let html = "<html><body><p>No markers</p></body></html>";
        let result = partition_page(html, &rules(), None);
        assert_eq!(result.parts.len(), 0);
        assert_eq!(result.parent_content, html);
    }

    #[test]
    fn unknown_part_id_uses_default_path() {
        let html = "<!-- s3d-part: unknown --><div>x</div><!-- s3d-part-end -->";
        let result = partition_page(html, &[], None);
        assert_eq!(result.parts.len(), 1);
        assert_eq!(result.parts[0].output_path, "parts/unknown.html");
    }

    #[test]
    fn iframe_attrs_applied() {
        let html = "<!-- s3d-part: header --><h1>H</h1><!-- s3d-part-end -->";
        let result = partition_page(html, &rules(), Some("loading=\"lazy\""));
        assert!(result.parent_content.contains("loading=\"lazy\""));
    }

    #[test]
    fn part_content_is_trimmed() {
        let html = "<!-- s3d-part: main -->\n  <p>hi</p>\n<!-- s3d-part-end -->";
        let result = partition_page(html, &rules(), None);
        assert_eq!(result.parts[0].content, "<p>hi</p>");
    }

    // ─────────────────────────────────────────────
    // s3d-iframe マーカー変換テスト (Issue #10)
    // ─────────────────────────────────────────────

    #[test]
    fn iframe_marker_basic_conversion() {
        let html = r#"<div><!-- s3d-iframe: price-123 src="parts/price-123.html" --></div>"#;
        let (out, markers) = replace_iframe_markers(html, None);

        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].id, "price-123");
        assert_eq!(markers[0].src, "parts/price-123.html");

        assert!(out.contains(r#"src="parts/price-123.html""#));
        assert!(out.contains(r#"data-s3d-id="price-123""#));
        // 元のマーカーコメントは消えている
        assert!(!out.contains("s3d-iframe:"));
    }

    #[test]
    fn iframe_marker_multiple_markers() {
        let html = concat!(
            r#"<!-- s3d-iframe: price-1 src="parts/price-1.html" -->"#,
            r#"<!-- s3d-iframe: stock-1 src="parts/stock-1.html" -->"#,
        );
        let (out, markers) = replace_iframe_markers(html, None);

        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].id, "price-1");
        assert_eq!(markers[1].id, "stock-1");
        assert!(out.contains("parts/price-1.html"));
        assert!(out.contains("parts/stock-1.html"));
    }

    #[test]
    fn iframe_marker_no_markers_returns_original() {
        let html = "<p>no markers here</p>";
        let (out, markers) = replace_iframe_markers(html, None);
        assert_eq!(out, html);
        assert!(markers.is_empty());
    }

    #[test]
    fn iframe_marker_custom_attrs() {
        let html = r#"<!-- s3d-iframe: x src="x.html" -->"#;
        let (out, _) = replace_iframe_markers(html, Some(r#"loading="lazy""#));
        assert!(out.contains(r#"loading="lazy""#));
    }

    #[test]
    fn iframe_marker_preserves_surrounding_html() {
        let html =
            r#"<header>TOP</header><!-- s3d-iframe: price src="p.html" --><footer>BOT</footer>"#;
        let (out, _) = replace_iframe_markers(html, None);
        assert!(out.starts_with("<header>TOP</header>"));
        assert!(out.ends_with("<footer>BOT</footer>"));
        assert!(out.contains("<iframe"));
    }

    #[test]
    fn iframe_marker_with_single_quote_src() {
        let html = "<!-- s3d-iframe: abc src='parts/abc.html' -->";
        let (out, markers) = replace_iframe_markers(html, None);
        assert_eq!(markers[0].src, "parts/abc.html");
        assert!(out.contains("parts/abc.html"));
    }

    #[test]
    fn both_markers_work_together() {
        let html = concat!(
            "<!-- s3d-part: header --><h1>H</h1><!-- s3d-part-end -->",
            r#"<p><!-- s3d-iframe: price src="parts/price.html" --></p>"#,
        );
        // まず s3d-part で分割
        let partition = partition_page(html, &rules(), None);
        assert_eq!(partition.parts.len(), 1);

        // 次に親ページ内の s3d-iframe を変換
        let (final_html, markers) = replace_iframe_markers(&partition.parent_content, None);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].id, "price");
        assert!(final_html.contains(r#"data-s3d-id="price""#));
    }
}
