//! インメモリキャッシュ制御モジュール
//!
//! `manifest` のアセットハッシュをキーにバイト列を保持する。
//! ブラウザの Cache API の代わりに、ネイティブ・Wasm 両環境で動く
//! シンプルな `HashMap` ベースのキャッシュを提供する。
//!
//! ## 設計方針
//! - キー: マニフェストのアセットキー（例: `"js/main.js"`）
//! - ハッシュが一致 → キャッシュヒット（ネットワーク取得スキップ）
//! - ハッシュが変化 → 古エントリを削除し新エントリを保存
//! - Service Worker 不使用・フレームワーク非依存

use std::collections::HashMap;

// ─────────────────────────────────────────────
// キャッシュエントリ
// ─────────────────────────────────────────────

/// キャッシュに保存されるアセットの1エントリ
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// アセットハッシュ（hex）—— キャッシュ有効性の判定に使う
    pub hash: String,
    /// レスポンスボディ（bytes）
    pub data: Vec<u8>,
}

// ─────────────────────────────────────────────
// AssetCache
// ─────────────────────────────────────────────

/// インメモリアセットキャッシュ
///
/// スレッドセーフにしたい場合は `Arc<Mutex<AssetCache>>` でラップする。
#[derive(Debug, Default)]
pub struct AssetCache {
    store: HashMap<String, CacheEntry>,
}

impl AssetCache {
    /// 新しい空のキャッシュを作成する
    pub fn new() -> Self {
        Self::default()
    }

    /// キャッシュヒット判定
    ///
    /// 指定キーのエントリが存在し、かつハッシュが一致すれば `Some(&[u8])` を返す。
    pub fn get(&self, key: &str, hash: &str) -> Option<&[u8]> {
        self.store.get(key).and_then(|e| {
            if e.hash == hash {
                Some(e.data.as_slice())
            } else {
                None
            }
        })
    }

    /// エントリを保存する
    ///
    /// 同じキーの古いエントリは自動的に上書き（削除 + 追加）される。
    pub fn put(&mut self, key: String, hash: String, data: Vec<u8>) {
        self.store.insert(key, CacheEntry { hash, data });
    }

    /// 指定キーのエントリを削除する
    pub fn evict(&mut self, key: &str) {
        self.store.remove(key);
    }

    /// 古いハッシュのエントリをすべて削除する
    ///
    /// `current_hashes` に含まれないキー、または含まれていてもハッシュが
    /// 異なるエントリを削除する。新しい manifest への移行後に呼ぶ。
    pub fn evict_stale(&mut self, current_hashes: &HashMap<String, String>) {
        self.store.retain(|key, entry| {
            current_hashes
                .get(key)
                .map(|h| h == &entry.hash)
                .unwrap_or(false)
        });
    }

    /// キャッシュに含まれるエントリ数
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// キャッシュが空かどうか
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_on_matching_hash() {
        let mut cache = AssetCache::new();
        cache.put(
            "js/main.js".to_string(),
            "abc".to_string(),
            b"data".to_vec(),
        );

        let hit = cache.get("js/main.js", "abc");
        assert_eq!(hit, Some(b"data".as_slice()));
    }

    #[test]
    fn cache_miss_on_hash_mismatch() {
        let mut cache = AssetCache::new();
        cache.put(
            "js/main.js".to_string(),
            "abc".to_string(),
            b"data".to_vec(),
        );

        // ハッシュが変わったらミス
        assert!(cache.get("js/main.js", "xyz").is_none());
    }

    #[test]
    fn cache_miss_on_unknown_key() {
        let cache = AssetCache::new();
        assert!(cache.get("not/exist.js", "abc").is_none());
    }

    #[test]
    fn cache_put_overwrites_old_entry() {
        let mut cache = AssetCache::new();
        cache.put("a.js".to_string(), "v1".to_string(), b"old".to_vec());
        cache.put("a.js".to_string(), "v2".to_string(), b"new".to_vec());

        assert!(cache.get("a.js", "v1").is_none());
        assert_eq!(cache.get("a.js", "v2"), Some(b"new".as_slice()));
    }

    #[test]
    fn evict_removes_entry() {
        let mut cache = AssetCache::new();
        cache.put("a.js".to_string(), "v1".to_string(), b"x".to_vec());
        cache.evict("a.js");
        assert!(cache.is_empty());
    }

    #[test]
    fn evict_stale_removes_outdated_entries() {
        let mut cache = AssetCache::new();
        cache.put("a.js".to_string(), "v1".to_string(), b"a".to_vec());
        cache.put("b.js".to_string(), "v2".to_string(), b"b".to_vec());
        cache.put("c.js".to_string(), "v3".to_string(), b"c".to_vec());

        // a.js は同じハッシュ → 残す
        // b.js はハッシュ変化 → 削除
        // c.js はキーなし → 削除
        let mut current = HashMap::new();
        current.insert("a.js".to_string(), "v1".to_string());
        current.insert("b.js".to_string(), "v2_new".to_string());

        cache.evict_stale(&current);

        assert!(cache.get("a.js", "v1").is_some());
        assert!(cache.get("b.js", "v2").is_none());
        assert!(cache.get("c.js", "v3").is_none());
        assert_eq!(cache.len(), 1);
    }
}
