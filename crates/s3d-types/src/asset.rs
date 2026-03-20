use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 収集済みアセット（ハッシュ計算前）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedAsset {
    /// マニフェスト内でアセットを識別するキー
    pub key: String,
    /// ディスク上の絶対パス
    pub absolute_path: PathBuf,
    /// ファイルサイズ（バイト）
    pub size: u64,
}

/// ハッシュ済みアセット（CDN アップロード前）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashedAsset {
    /// マニフェスト内でアセットを識別するキー
    pub key: String,
    /// ディスク上の絶対パス
    pub absolute_path: PathBuf,
    /// ファイルサイズ（バイト）
    pub size: u64,
    /// ファイル内容の SHA-256 ハッシュ（hex）
    pub hash: String,
    /// ハッシュを含むファイル名（例: `main.abc1234.js`）
    pub hashed_filename: String,
    /// ハッシュを含むアセットキー
    pub hashed_key: String,
}

/// 前バージョンとの差分種別
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetDiff {
    /// 新規追加されたアセット
    Added,
    /// 内容が変更されたアセット
    Modified,
    /// 削除されたアセット
    Deleted,
    /// 変更なし
    Unchanged,
}

/// アセットの配信戦略
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetStrategy {
    /// 静的ファイルとして直接配信
    Static,
    /// iframe 経由で配信
    Iframe,
    /// CDN 経由で配信
    Cdn,
}
