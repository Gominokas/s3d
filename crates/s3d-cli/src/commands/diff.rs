//! `s3d diff` — 2 つのマニフェストを比較してコンソールに表示するコマンド

use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use s3d_deploy::diff::{diff_manifests, DiffEntry};
use s3d_types::asset::AssetDiff;
use s3d_types::manifest::DeployManifest;

/// `s3d diff` を実行する
///
/// `old_path`: 旧 manifest.json のパス（`None` = 初回デプロイ扱い）
/// `new_path`: 新 manifest.json のパス
pub fn run(old_path: Option<&Path>, new_path: &Path) -> Result<()> {
    println!("{}", "s3d diff — マニフェスト差分".bold().cyan());

    let new_manifest = load_manifest(new_path)?;
    let old_manifest = old_path.map(load_manifest).transpose()?;

    let entries = diff_manifests(old_manifest.as_ref(), &new_manifest);

    print_diff(&entries);

    Ok(())
}

fn load_manifest(path: &Path) -> Result<DeployManifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("マニフェストの読み込み失敗: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("マニフェストのパース失敗: {}", path.display()))
}

pub fn print_diff(entries: &[DiffEntry]) {
    let mut added = 0usize;
    let mut modified = 0usize;
    let mut deleted = 0usize;
    let mut unchanged = 0usize;

    for e in entries {
        match e.diff {
            AssetDiff::Added => {
                println!("  {} {}", "+".green().bold(), e.key);
                added += 1;
            }
            AssetDiff::Modified => {
                println!("  {} {}", "~".yellow().bold(), e.key);
                modified += 1;
            }
            AssetDiff::Deleted => {
                println!("  {} {}", "-".red().bold(), e.key);
                deleted += 1;
            }
            AssetDiff::Unchanged => {
                unchanged += 1;
            }
        }
    }

    println!();
    println!(
        "  {} 追加  {} 変更  {} 削除  {} 変更なし",
        added.to_string().green().bold(),
        modified.to_string().yellow().bold(),
        deleted.to_string().red().bold(),
        unchanged.to_string().dimmed(),
    );
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use s3d_deploy::diff::DiffEntry;
    use s3d_types::asset::AssetDiff;

    #[test]
    fn test_print_diff_counts() {
        let entries = vec![
            DiffEntry {
                key: "a.js".to_string(),
                diff: AssetDiff::Added,
            },
            DiffEntry {
                key: "b.css".to_string(),
                diff: AssetDiff::Modified,
            },
            DiffEntry {
                key: "c.png".to_string(),
                diff: AssetDiff::Deleted,
            },
            DiffEntry {
                key: "d.glb".to_string(),
                diff: AssetDiff::Unchanged,
            },
        ];
        // should not panic
        print_diff(&entries);
    }

    #[test]
    fn test_run_with_files() {
        use s3d_types::manifest::{AssetEntry, DeployManifest};
        use std::collections::HashMap;
        use tempfile::NamedTempFile;

        let make_manifest = |keys: &[&str]| -> String {
            let mut assets = HashMap::new();
            for k in keys {
                assets.insert(
                    k.to_string(),
                    AssetEntry {
                        url: format!("https://cdn.example.com/{k}"),
                        size: 100,
                        hash: "aabbccdd".to_string(),
                        content_type: "text/plain".to_string(),
                        dependencies: None,
                    },
                );
            }
            let m = DeployManifest {
                schema_version: 1,
                version: "1.0.0".to_string(),
                build_time: "2026-01-01T00:00:00Z".to_string(),
                assets,
            };
            serde_json::to_string_pretty(&m).unwrap()
        };

        let old_file = NamedTempFile::new().unwrap();
        let new_file = NamedTempFile::new().unwrap();
        std::fs::write(old_file.path(), make_manifest(&["a.js", "b.css"])).unwrap();
        std::fs::write(new_file.path(), make_manifest(&["a.js", "c.glb"])).unwrap();

        run(Some(old_file.path()), new_file.path()).unwrap();
    }
}
