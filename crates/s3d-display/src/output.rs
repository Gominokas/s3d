//! 出力ディレクトリへの書き出しモジュール
//!
//! 生成した HTML ファイル群を指定ディレクトリに書き出す。
//!
//! ## ディレクトリ構造
//! ```text
//! output/
//! ├─ index.html          ← 親ページ
//! ├─ parts/
//! │   ├─ header.html
//! │   ├─ main.html
//! │   └─ footer.html
//! └─ assets/             ← 静的ファイル
//! ```

use std::path::{Path, PathBuf};

use thiserror::Error;

// ─────────────────────────────────────────────
// エラー型
// ─────────────────────────────────────────────

/// output モジュールのエラー型
#[derive(Debug, Error)]
pub enum OutputError {
    #[error("I/O error writing `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ─────────────────────────────────────────────
// OutputFile — 書き出し対象1件
// ─────────────────────────────────────────────

/// 書き出し対象のファイル1件
#[derive(Debug, Clone)]
pub struct OutputFile {
    /// 出力先の相対パス（`output_dir` からの相対）
    pub relative_path: String,
    /// ファイル内容
    pub content: String,
    /// Cache-Control ヘッダー値（deploy 時に利用）
    pub cache_control: Option<String>,
}

// ─────────────────────────────────────────────
// write_output_files
// ─────────────────────────────────────────────

/// `OutputFile` のリストをディスクに書き出す
///
/// - `output_dir`: 書き出し先のルートディレクトリ
/// - `files`: 書き出すファイルのリスト
///
/// 各ファイルの親ディレクトリが存在しない場合は自動的に作成する。
pub fn write_output_files(
    output_dir: &Path,
    files: &[OutputFile],
) -> Result<Vec<PathBuf>, OutputError> {
    let mut written: Vec<PathBuf> = Vec::new();

    for file in files {
        let dest = output_dir.join(&file.relative_path);

        // 親ディレクトリを作成
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|source| OutputError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        // ファイル書き込み
        std::fs::write(&dest, &file.content).map_err(|source| OutputError::Io {
            path: dest.clone(),
            source,
        })?;

        written.push(dest);
    }

    Ok(written)
}

/// `OutputFile` のリストを生成して返す（ディスク書き込みなし）
///
/// テスト・プレビューモード用。`index.html` を先頭に保証する。
pub fn collect_output_files(
    parent_html: String,
    parts: Vec<crate::iframe::Part>,
    part_htmls: Vec<String>,
    index_name: &str,
) -> Vec<OutputFile> {
    let mut files: Vec<OutputFile> = Vec::new();

    // 親ページ
    files.push(OutputFile {
        relative_path: index_name.to_string(),
        content: parent_html,
        cache_control: None,
    });

    // パーツ
    for (part, html) in parts.iter().zip(part_htmls.iter()) {
        files.push(OutputFile {
            relative_path: part.output_path.clone(),
            content: html.clone(),
            cache_control: part.cache_control.clone(),
        });
    }

    files
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iframe::Part;
    use tempfile::TempDir;

    fn sample_parts() -> Vec<Part> {
        vec![
            Part {
                id: "header".to_string(),
                content: "<h1>H</h1>".to_string(),
                output_path: "parts/header.html".to_string(),
                cache_control: Some("max-age=3600".to_string()),
            },
            Part {
                id: "footer".to_string(),
                content: "<footer>F</footer>".to_string(),
                output_path: "parts/footer.html".to_string(),
                cache_control: None,
            },
        ]
    }

    #[test]
    fn write_creates_files_and_dirs() {
        let tmp = TempDir::new().unwrap();
        let files = vec![
            OutputFile {
                relative_path: "index.html".to_string(),
                content: "<html>parent</html>".to_string(),
                cache_control: None,
            },
            OutputFile {
                relative_path: "parts/header.html".to_string(),
                content: "<html>header</html>".to_string(),
                cache_control: Some("max-age=3600".to_string()),
            },
        ];

        let written = write_output_files(tmp.path(), &files).unwrap();
        assert_eq!(written.len(), 2);
        assert!(tmp.path().join("index.html").exists());
        assert!(tmp.path().join("parts/header.html").exists());

        let content = std::fs::read_to_string(tmp.path().join("index.html")).unwrap();
        assert!(content.contains("parent"));
    }

    #[test]
    fn collect_output_files_index_first() {
        let parts = sample_parts();
        let part_htmls = vec![
            "<html>header</html>".to_string(),
            "<html>footer</html>".to_string(),
        ];
        let files = collect_output_files(
            "<html>parent</html>".to_string(),
            parts,
            part_htmls,
            "index.html",
        );

        assert_eq!(files[0].relative_path, "index.html");
        assert_eq!(files[1].relative_path, "parts/header.html");
        assert_eq!(files[2].relative_path, "parts/footer.html");
        assert_eq!(files[1].cache_control, Some("max-age=3600".to_string()));
        assert!(files[2].cache_control.is_none());
    }

    #[test]
    fn output_dir_created_if_not_exists() {
        let tmp = TempDir::new().unwrap();
        let deep_dir = tmp.path().join("a/b/c");
        let files = vec![OutputFile {
            relative_path: "a/b/c/index.html".to_string(),
            content: "x".to_string(),
            cache_control: None,
        }];
        write_output_files(tmp.path(), &files).unwrap();
        assert!(deep_dir.join("index.html").exists());
    }
}
