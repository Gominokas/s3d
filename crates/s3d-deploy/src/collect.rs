//! アセットファイルの収集モジュール
//!
//! 指定ディレクトリを再帰走査し、glob パターン・ファイルサイズ制限を適用して
//! [`CollectedAsset`] のリストを返す。

use std::path::{Path, PathBuf};

use globset::{GlobSet, GlobSetBuilder};
use s3d_types::asset::CollectedAsset;
use thiserror::Error;
use walkdir::WalkDir;

/// collect モジュールのエラー型
#[derive(Debug, Error)]
pub enum CollectError {
    #[error("directory walk error: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("glob pattern error `{pattern}`: {source}")]
    GlobPattern {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    #[error("I/O error at `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// ファイルサイズ上限を文字列からバイト数に変換する。
///
/// 対応フォーマット: `"10MB"`, `"512KB"`, `"1GB"`, `"4096"` (bytes)
fn parse_max_file_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        (&s[..pos], s[pos..].to_ascii_uppercase())
    } else {
        (s, String::new())
    };
    let n: u64 = num.trim().parse().ok()?;
    let multiplier = match unit.as_str() {
        "KB" | "K" => 1_024,
        "MB" | "M" => 1_024 * 1_024,
        "GB" | "G" => 1_024 * 1_024 * 1_024,
        "" => 1,
        _ => return None,
    };
    Some(n * multiplier)
}

/// glob パターン文字列のリストから [`GlobSet`] を構築する。
///
/// パターンは case-insensitive で照合する。
fn build_globset(patterns: &[String]) -> Result<Option<GlobSet>, CollectError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(
            globset::GlobBuilder::new(p)
                .case_insensitive(true)
                .build()
                .map_err(|source| CollectError::GlobPattern {
                    pattern: p.clone(),
                    source,
                })?,
        );
    }
    Ok(Some(builder.build().map_err(|source| {
        CollectError::GlobPattern {
            pattern: patterns.join(", "),
            source,
        }
    })?))
}

/// アセット収集オプション
#[derive(Debug, Clone, Default)]
pub struct CollectOptions {
    /// 除外する glob パターン（case-insensitive）
    pub ignore: Vec<String>,
    /// 明示的に含める glob パターン。空の場合はすべて対象
    pub include: Vec<String>,
    /// 1 ファイルあたりの最大サイズ（例: `"10MB"`）。超過ファイルはスキップ
    pub max_file_size: Option<String>,
}

/// `root_dir` 以下のファイルを再帰走査してアセットを収集する。
///
/// - `key` はルートディレクトリからの相対パス（スラッシュ区切り）
/// - `ignore` パターンに一致するファイルはスキップ
/// - `include` パターンが指定されている場合はいずれかに一致するファイルのみ対象
/// - `max_file_size` を超えるファイルはスキップ
pub fn collect(
    root_dir: &Path,
    opts: &CollectOptions,
) -> Result<Vec<CollectedAsset>, CollectError> {
    let ignore_set = build_globset(&opts.ignore)?;
    let include_set = build_globset(&opts.include)?;
    let max_bytes = opts.max_file_size.as_deref().and_then(parse_max_file_size);

    let mut assets = Vec::new();

    for entry in WalkDir::new(root_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path().to_path_buf();

        // ルートからの相対パス（スラッシュ区切り）をキーにする
        let rel = abs_path
            .strip_prefix(root_dir)
            .unwrap_or(&abs_path)
            .to_string_lossy();
        // Windows 対応: バックスラッシュをスラッシュに正規化
        let key = rel.replace('\\', "/");

        // ignore フィルタ
        if let Some(ref gs) = ignore_set {
            if gs.is_match(&key) {
                continue;
            }
        }

        // include フィルタ
        if let Some(ref gs) = include_set {
            if !gs.is_match(&key) {
                continue;
            }
        }

        // ファイルサイズ取得
        let metadata = std::fs::metadata(&abs_path).map_err(|source| CollectError::Io {
            path: abs_path.clone(),
            source,
        })?;
        let size = metadata.len();

        // maxFileSize フィルタ
        if let Some(max) = max_bytes {
            if size > max {
                continue;
            }
        }

        assets.push(CollectedAsset {
            key,
            absolute_path: abs_path,
            size,
        });
    }

    // 再現性のためキー順にソート
    assets.sort_by(|a, b| a.key.cmp(&b.key));

    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_file(dir: &Path, rel: &str, content: &[u8]) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn collect_all_files() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "a.js", b"console.log('a')");
        make_file(tmp.path(), "sub/b.css", b"body{}");

        let assets = collect(tmp.path(), &CollectOptions::default()).unwrap();
        let keys: Vec<_> = assets.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains(&"a.js"));
        assert!(keys.contains(&"sub/b.css"));
    }

    #[test]
    fn collect_respects_ignore() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "main.js", b"x");
        make_file(tmp.path(), "main.js.map", b"{}");

        let opts = CollectOptions {
            ignore: vec!["**/*.map".to_string()],
            ..Default::default()
        };
        let assets = collect(tmp.path(), &opts).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].key, "main.js");
    }

    #[test]
    fn collect_respects_include() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "a.js", b"x");
        make_file(tmp.path(), "b.css", b"x");
        make_file(tmp.path(), "c.txt", b"x");

        let opts = CollectOptions {
            include: vec!["**/*.js".to_string(), "**/*.css".to_string()],
            ..Default::default()
        };
        let assets = collect(tmp.path(), &opts).unwrap();
        let keys: Vec<_> = assets.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains(&"a.js"));
        assert!(keys.contains(&"b.css"));
        assert!(!keys.contains(&"c.txt"));
    }

    #[test]
    fn collect_respects_max_file_size() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "small.bin", &[0u8; 100]);
        make_file(tmp.path(), "large.bin", &[0u8; 2000]);

        let opts = CollectOptions {
            max_file_size: Some("1KB".to_string()),
            ..Default::default()
        };
        let assets = collect(tmp.path(), &opts).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].key, "small.bin");
    }

    #[test]
    fn collect_ignore_is_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "Photo.JPG", b"img");
        make_file(tmp.path(), "doc.pdf", b"pdf");

        let opts = CollectOptions {
            ignore: vec!["**/*.jpg".to_string()],
            ..Default::default()
        };
        let assets = collect(tmp.path(), &opts).unwrap();
        // .JPG は case-insensitive で ignore される
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].key, "doc.pdf");
    }

    #[test]
    fn parse_max_file_size_variants() {
        assert_eq!(parse_max_file_size("10MB"), Some(10 * 1024 * 1024));
        assert_eq!(parse_max_file_size("512KB"), Some(512 * 1024));
        assert_eq!(parse_max_file_size("1GB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_max_file_size("4096"), Some(4096));
        assert_eq!(parse_max_file_size("bad"), None);
    }
}
