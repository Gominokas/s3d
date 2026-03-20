use serde::{Deserialize, Serialize};

/// CDN プロバイダーの種別
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CdnProvider {
    CloudflareR2,
}

/// ページのデプロイ設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PagesConfig {
    pub output_dir: String,
    pub custom_domain: Option<String>,
}

/// CDN の設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdnConfig {
    pub provider: CdnProvider,
    pub bucket: String,
    pub base_url: String,
    pub region: Option<String>,
}

/// アセットのデプロイ設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetsDeployConfig {
    pub immediate_dir: String,
    pub deferred_dir: String,
    pub hash_length: Option<u32>,
    pub max_file_size: Option<String>,
    pub ignore: Option<Vec<String>>,
    pub include: Option<Vec<String>>,
}

/// デプロイ全体の設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployConfig {
    pub pages: PagesConfig,
    pub cdn: CdnConfig,
    pub assets: AssetsDeployConfig,
    pub old_version_retention: Option<u32>,
    pub old_version_max_age: Option<String>,
}

/// ローダーの表示設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderDisplayConfig {
    pub concurrency: Option<u32>,
    pub retry_count: Option<u32>,
    pub retry_base_delay: Option<u64>,
    pub timeout: Option<u64>,
}

/// 表示設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayConfig {
    pub loader: LoaderDisplayConfig,
}

/// ドラフトプレビューの設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftPreviewConfig {
    pub expires_in: Option<String>,
}

/// ドラフト設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftConfig {
    pub preview: DraftPreviewConfig,
}

/// s3d プロジェクト全体の設定
///
/// TypeScript の `Static3dConfig` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3dConfig {
    pub schema_version: u32,
    pub project: String,
    pub deploy: Option<DeployConfig>,
    pub display: Option<DisplayConfig>,
    pub draft: Option<DraftConfig>,
}
