use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// デプロイマニフェスト内の個々のアセットエントリ
///
/// TypeScript の `AssetEntry` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetEntry {
    pub url: String,
    pub size: u64,
    pub hash: String,
    pub content_type: String,
    pub dependencies: Option<Vec<String>>,
}

/// strategy.json の reload セクション
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StrategyReload {
    pub trigger: String,
    pub strategy: String,
}

/// strategies セクションの個々のエントリ
///
/// `src/assetsStrategy/<name>/strategy.json` から読み込まれる。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEntry {
    /// 対象ファイル一覧 (src/assets/ 相対パス)
    pub files: Vec<String>,
    /// 初期ロード対象かどうか
    pub initial: bool,
    /// キャッシュを有効にするか
    pub cache: bool,
    /// キャッシュの最大有効期間 (例: "7d")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age: Option<String>,
    /// リロード設定
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reload: Option<StrategyReload>,
}

/// デプロイマニフェスト
///
/// TypeScript の `DeployManifest` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployManifest {
    pub schema_version: u32,
    pub version: String,
    pub build_time: String,
    pub assets: HashMap<String, AssetEntry>,
    /// 配信戦略マップ（strategy 名 → StrategyEntry）
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub strategies: HashMap<String, StrategyEntry>,
}
