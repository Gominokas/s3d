//! manifest 差分検知モジュール
//!
//! キャッシュ済みの旧 manifest と CDN から取得した新 manifest を比較し、
//! 変更されたアセットだけを取得対象として返す。
//!
//! `s3d-deploy` の `diff.rs` はデプロイ時の比較（ファイルシステム → CDN）に対し、
//! こちらはランタイム時のクライアント側での比較（キャッシュ → CDN manifest）。

use std::collections::HashMap;

use s3d_types::asset::AssetDiff;
use s3d_types::manifest::DeployManifest;

// ─────────────────────────────────────────────
// DiffEntry
// ─────────────────────────────────────────────

/// manifest 差分の1エントリ
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// マニフェストキー（例: `"js/main.js"`）
    pub key: String,
    /// 差分種別
    pub diff: AssetDiff,
    /// 新 manifest の URL（Deleted は None）
    pub url: Option<String>,
    /// 新 manifest のハッシュ（Deleted は None）
    pub hash: Option<String>,
    /// ファイルサイズ（Deleted は 0）
    pub size: u64,
}

// ─────────────────────────────────────────────
// diff_manifests
// ─────────────────────────────────────────────

/// 旧 manifest（キャッシュ）と新 manifest（CDN）を比較して差分リストを返す
///
/// - `old`: キャッシュ済みの manifest。`None` の場合は「初回デプロイ」扱い（全アセットが Added）
/// - `new`: CDN から取得した最新 manifest
pub fn diff_manifests(old: Option<&DeployManifest>, new: &DeployManifest) -> Vec<DiffEntry> {
    let old_assets: HashMap<&str, _> = old
        .map(|m| m.assets.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default();

    let mut entries: Vec<DiffEntry> = Vec::new();

    // 新 manifest を走査: Added / Modified / Unchanged を判定
    for (key, new_entry) in &new.assets {
        let diff = match old_assets.get(key.as_str()) {
            None => AssetDiff::Added,
            Some(old_entry) => {
                if old_entry.hash == new_entry.hash {
                    AssetDiff::Unchanged
                } else {
                    AssetDiff::Modified
                }
            }
        };

        entries.push(DiffEntry {
            key: key.clone(),
            diff,
            url: Some(new_entry.url.clone()),
            hash: Some(new_entry.hash.clone()),
            size: new_entry.size,
        });
    }

    // 旧 manifest にあって新 manifest にないもの → Deleted
    for key in old_assets.keys() {
        if !new.assets.contains_key(*key) {
            entries.push(DiffEntry {
                key: key.to_string(),
                diff: AssetDiff::Deleted,
                url: None,
                hash: None,
                size: 0,
            });
        }
    }

    // 安定した順序でソート（テストの再現性向上）
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

/// 取得が必要なエントリだけを返す（Added + Modified のみ）
pub fn needs_fetch(entries: &[DiffEntry]) -> Vec<&DiffEntry> {
    entries
        .iter()
        .filter(|e| matches!(e.diff, AssetDiff::Added | AssetDiff::Modified))
        .collect()
}

/// キャッシュから削除すべきエントリだけを返す（Deleted + Modified）
pub fn needs_evict(entries: &[DiffEntry]) -> Vec<&DiffEntry> {
    entries
        .iter()
        .filter(|e| matches!(e.diff, AssetDiff::Deleted | AssetDiff::Modified))
        .collect()
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use s3d_types::manifest::AssetEntry;

    fn make_manifest(entries: Vec<(&str, &str, &str)>) -> DeployManifest {
        let mut assets = HashMap::new();
        for (key, url, hash) in entries {
            assets.insert(
                key.to_string(),
                AssetEntry {
                    url: url.to_string(),
                    size: 100,
                    hash: hash.to_string(),
                    content_type: "application/javascript".to_string(),
                    dependencies: None,
                },
            );
        }
        DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
        }
    }

    #[test]
    fn first_deploy_all_added() {
        let new = make_manifest(vec![
            ("js/main.js", "https://cdn/js/main.abc.js", "abc"),
            ("style.css", "https://cdn/style.xyz.css", "xyz"),
        ]);
        let entries = diff_manifests(None, &new);
        assert!(entries.iter().all(|e| e.diff == AssetDiff::Added));
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn unchanged_files_detected() {
        let old = make_manifest(vec![("js/main.js", "https://cdn/js/main.abc.js", "abc")]);
        let new = make_manifest(vec![("js/main.js", "https://cdn/js/main.abc.js", "abc")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries[0].diff, AssetDiff::Unchanged);
    }

    #[test]
    fn modified_file_detected() {
        let old = make_manifest(vec![("js/main.js", "https://cdn/js/main.abc.js", "abc")]);
        let new = make_manifest(vec![("js/main.js", "https://cdn/js/main.def.js", "def")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries[0].diff, AssetDiff::Modified);
        assert_eq!(entries[0].hash, Some("def".to_string()));
    }

    #[test]
    fn deleted_file_detected() {
        let old = make_manifest(vec![
            ("js/main.js", "https://cdn/js/main.abc.js", "abc"),
            ("old.js", "https://cdn/old.xyz.js", "xyz"),
        ]);
        let new = make_manifest(vec![("js/main.js", "https://cdn/js/main.abc.js", "abc")]);
        let entries = diff_manifests(Some(&old), &new);
        let deleted: Vec<_> = entries
            .iter()
            .filter(|e| e.diff == AssetDiff::Deleted)
            .collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].key, "old.js");
    }

    #[test]
    fn needs_fetch_filters_added_and_modified() {
        let old = make_manifest(vec![
            ("a.js", "https://cdn/a.v1.js", "v1"),
            ("b.js", "https://cdn/b.v1.js", "v1"),
        ]);
        let new = make_manifest(vec![
            ("a.js", "https://cdn/a.v2.js", "v2"), // Modified
            ("b.js", "https://cdn/b.v1.js", "v1"), // Unchanged
            ("c.js", "https://cdn/c.v1.js", "v1"), // Added
        ]);
        let entries = diff_manifests(Some(&old), &new);
        let fetch = needs_fetch(&entries);
        assert_eq!(fetch.len(), 2);
        let keys: Vec<&str> = fetch.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"a.js"));
        assert!(keys.contains(&"c.js"));
    }

    #[test]
    fn needs_evict_filters_deleted_and_modified() {
        let old = make_manifest(vec![
            ("a.js", "https://cdn/a.v1.js", "v1"), // Modified
            ("b.js", "https://cdn/b.v1.js", "v1"), // Deleted
        ]);
        let new = make_manifest(vec![
            ("a.js", "https://cdn/a.v2.js", "v2"), // Modified
        ]);
        let entries = diff_manifests(Some(&old), &new);
        let evict = needs_evict(&entries);
        assert_eq!(evict.len(), 2);
    }
}
