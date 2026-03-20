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
}
