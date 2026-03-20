use serde::{Deserialize, Serialize};

/// HTTP レスポンスの受信形式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseType {
    ArrayBuffer,
    Blob,
    Json,
    Text,
}

/// ロードエラーの種別
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadErrorKind {
    Network,
    Timeout,
    Integrity,
    NotFound,
    Abort,
    Unknown,
}

/// ローダーオプション
///
/// TypeScript の `LoaderOptions` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderOptions {
    pub concurrency: Option<u32>,
    pub retry_count: Option<u32>,
    pub retry_delay: Option<u64>,
    pub timeout: Option<u64>,
    pub integrity: Option<bool>,
}

/// 個別アセットのロードオプション
///
/// TypeScript の `LoadOptions` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadOptions {
    pub response_type: Option<ResponseType>,
}

/// 複数アセットの一括ロードオプション
///
/// TypeScript の `LoadAllOptions` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAllOptions {
    pub concurrency: Option<u32>,
    pub retry_count: Option<u32>,
    pub retry_delay: Option<u64>,
    pub timeout: Option<u64>,
}

/// ロード進捗イベント
///
/// TypeScript の `ProgressEvent` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub loaded: u64,
    pub total: u64,
    pub asset: String,
    pub completed_count: u64,
    pub total_count: u64,
}

/// ロードエラー
///
/// TypeScript の `LoadError` に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
    #[serde(rename = "type")]
    pub kind: LoadErrorKind,
    pub key: String,
    pub url: String,
    pub cause: Option<String>,
    pub status_code: Option<u16>,
}

/// アセットロード結果の型エイリアス
pub type AssetResult<T> = Result<T, LoadError>;
