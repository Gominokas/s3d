//! マニフェスト差分計算モジュール
//!
//! 旧バージョンと新バージョンの [`DeployManifest`] を比較し、
//! 各アセットキーごとに [`DiffEntry`] を返す。
//!
//! R2/S3 へのアップロード判定に使用する。
//! - `Added`   : 新バージョンにのみ存在 → アップロード必要
//! - `Modified`: 両方に存在するがハッシュが異なる → アップロード必要
//! - `Deleted` : 旧バージョンにのみ存在 → 削除候補
//! - `Unchanged`: 両方に存在しハッシュ AND URL が同一 → スキップ可

use s3d_types::asset::AssetDiff;
use s3d_types::manifest::DeployManifest;

/// 差分の 1 エントリ
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// アセットキー（ハッシュ付き）
    pub key: String,
    /// 差分種別
    pub diff: AssetDiff,
}

/// 旧マニフェスト（`None` = 初回デプロイ）と新マニフェストを比較して差分リストを返す。
///
/// 結果はキーのアルファベット順にソートされる。
pub fn diff_manifests(old: Option<&DeployManifest>, new: &DeployManifest) -> Vec<DiffEntry> {
    let mut entries: Vec<DiffEntry> = Vec::new();

    // 旧マニフェストが存在しない場合はすべて Added
    let old_assets = match old {
        Some(m) => &m.assets,
        None => {
            let mut result: Vec<DiffEntry> = new
                .assets
                .keys()
                .map(|k| DiffEntry {
                    key: k.clone(),
                    diff: AssetDiff::Added,
                })
                .collect();
            result.sort_by(|a, b| a.key.cmp(&b.key));
            return result;
        }
    };

    // 新バージョン側を走査
    for (key, new_entry) in &new.assets {
        let diff = match old_assets.get(key) {
            // hash AND url の両方が一致する場合のみ Unchanged
            // url が変わった場合（例: ハッシュなし→ハッシュ付きへの移行）もアップロード対象にする
            Some(old_entry)
                if old_entry.hash == new_entry.hash && old_entry.url == new_entry.url =>
            {
                AssetDiff::Unchanged
            }
            Some(_) => AssetDiff::Modified,
            None => AssetDiff::Added,
        };
        entries.push(DiffEntry {
            key: key.clone(),
            diff,
        });
    }

    // 旧バージョンにしか存在しないキー → Deleted
    for key in old_assets.keys() {
        if !new.assets.contains_key(key) {
            entries.push(DiffEntry {
                key: key.clone(),
                diff: AssetDiff::Deleted,
            });
        }
    }

    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

/// 差分リストからアップロードが必要なキー（Added / Modified）を返す。
pub fn needs_upload(entries: &[DiffEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter(|e| matches!(e.diff, AssetDiff::Added | AssetDiff::Modified))
        .map(|e| e.key.as_str())
        .collect()
}

/// 差分リストから削除候補のキー（Deleted）を返す。
pub fn needs_delete(entries: &[DiffEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter(|e| e.diff == AssetDiff::Deleted)
        .map(|e| e.key.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3d_types::manifest::AssetEntry;
    use std::collections::HashMap;

    fn make_manifest(assets: &[(&str, &str)]) -> DeployManifest {
        let map: HashMap<String, AssetEntry> = assets
            .iter()
            .map(|(key, hash)| {
                (
                    key.to_string(),
                    AssetEntry {
                        url: format!("https://cdn.test/{key}"),
                        size: 1,
                        hash: hash.to_string(),
                        content_type: "application/octet-stream".to_string(),
                        dependencies: None,
                    },
                )
            })
            .collect();
        DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: map,
            strategies: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn diff_first_deploy_all_added() {
        let new = make_manifest(&[("a.js", "aaaa"), ("b.css", "bbbb")]);
        let entries = diff_manifests(None, &new);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.diff == AssetDiff::Added));
    }

    #[test]
    fn diff_unchanged() {
        let old = make_manifest(&[("a.js", "aaaa")]);
        let new = make_manifest(&[("a.js", "aaaa")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].diff, AssetDiff::Unchanged);
    }

    #[test]
    fn diff_url_changed_same_hash_is_modified() {
        // Issue #37: hash が同じでも url が変わったら Modified になることを確認
        // 例: 論理キー名のまま push 済み → ハッシュ付き URL に変わった場合
        use s3d_types::manifest::AssetEntry;

        let mut old_assets = HashMap::new();
        old_assets.insert(
            "assets/cake-3d.bin".to_string(),
            AssetEntry {
                url: "https://cdn.test/assets/cake-3d.bin".to_string(), // ハッシュなし URL
                size: 1,
                hash: "30e14955".to_string(),
                content_type: "application/octet-stream".to_string(),
                dependencies: None,
            },
        );
        let old = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: old_assets,
            strategies: HashMap::new(),
        };

        let mut new_assets = HashMap::new();
        new_assets.insert(
            "assets/cake-3d.bin".to_string(),
            AssetEntry {
                url: "https://cdn.test/assets/cake-3d.30e14955.bin".to_string(), // ハッシュ付き URL
                size: 1,
                hash: "30e14955".to_string(), // hash は同じ
                content_type: "application/octet-stream".to_string(),
                dependencies: None,
            },
        );
        let new = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: new_assets,
            strategies: HashMap::new(),
        };

        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries.len(), 1);
        // hash が同じでも url が違うので Modified
        assert_eq!(entries[0].diff, AssetDiff::Modified,
            "hash が同一でも url が変わった場合は Modified であるべき");
    }

    #[test]
    fn diff_unchanged_requires_both_hash_and_url() {
        // hash AND url の両方が一致する場合のみ Unchanged
        use s3d_types::manifest::AssetEntry;

        let make_entry = |url: &str, hash: &str| AssetEntry {
            url: url.to_string(),
            size: 1,
            hash: hash.to_string(),
            content_type: "application/octet-stream".to_string(),
            dependencies: None,
        };

        let mut assets = HashMap::new();
        assets.insert(
            "a.glb".to_string(),
            make_entry("https://cdn.test/a.abcd1234.glb", "abcd1234"),
        );
        let old = DeployManifest {
            schema_version: 1, version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets: assets.clone(), strategies: HashMap::new(),
        };
        let new = DeployManifest {
            schema_version: 1, version: "1.0.0".to_string(),
            build_time: "2026-01-01T00:00:00Z".to_string(),
            assets, strategies: HashMap::new(),
        };

        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries[0].diff, AssetDiff::Unchanged,
            "hash AND url が同一なら Unchanged");
    }

    #[test]
    fn diff_modified() {
        let old = make_manifest(&[("a.js", "aaaa")]);
        let new = make_manifest(&[("a.js", "bbbb")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries[0].diff, AssetDiff::Modified);
    }

    #[test]
    fn diff_deleted() {
        let old = make_manifest(&[("a.js", "aaaa"), ("b.css", "bbbb")]);
        let new = make_manifest(&[("a.js", "aaaa")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(entries.len(), 2);
        let deleted: Vec<_> = entries
            .iter()
            .filter(|e| e.diff == AssetDiff::Deleted)
            .collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].key, "b.css");
    }

    #[test]
    fn diff_mixed() {
        let old = make_manifest(&[
            ("keep.js", "same"),
            ("modify.css", "old"),
            ("remove.bin", "del"),
        ]);
        let new = make_manifest(&[
            ("keep.js", "same"),
            ("modify.css", "new"),
            ("added.glb", "new"),
        ]);
        let entries = diff_manifests(Some(&old), &new);

        let find = |key: &str| entries.iter().find(|e| e.key == key).map(|e| &e.diff);
        assert_eq!(find("keep.js"), Some(&AssetDiff::Unchanged));
        assert_eq!(find("modify.css"), Some(&AssetDiff::Modified));
        assert_eq!(find("remove.bin"), Some(&AssetDiff::Deleted));
        assert_eq!(find("added.glb"), Some(&AssetDiff::Added));
    }

    #[test]
    fn needs_upload_filters_correctly() {
        let old = make_manifest(&[("a.js", "old"), ("keep.js", "same")]);
        let new = make_manifest(&[("a.js", "new"), ("keep.js", "same"), ("b.css", "fresh")]);
        let entries = diff_manifests(Some(&old), &new);
        let mut upload = needs_upload(&entries);
        upload.sort();
        assert_eq!(upload, vec!["a.js", "b.css"]);
    }

    #[test]
    fn needs_delete_filters_correctly() {
        let old = make_manifest(&[("a.js", "aa"), ("old.bin", "bb")]);
        let new = make_manifest(&[("a.js", "aa")]);
        let entries = diff_manifests(Some(&old), &new);
        assert_eq!(needs_delete(&entries), vec!["old.bin"]);
    }
}
