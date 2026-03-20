use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{config::S3dConfig, manifest::DeployManifest};

/// HTML レンダリング結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlOutput {
    /// 出力先のパス（例: `index.html`）
    pub path: String,
    /// HTML コンテンツ
    pub content: String,
    /// Cache-Control ヘッダーの値（省略可）
    pub cache_control: Option<String>,
}

/// レンダリング時のコンテキスト情報
pub struct RenderContext<'a> {
    pub config: &'a S3dConfig,
    pub manifest: &'a DeployManifest,
}

/// 表示プラグインのトレイト
///
/// HTML ページを生成する実装を提供する。
/// レンダリングは CPU バウンドなので同期のままとする。
pub trait DisplayPlugin {
    fn render(&self, context: &RenderContext) -> Vec<HtmlOutput>;
}

/// ストレージ操作の結果型
pub type StorageResult<T> = Result<T, StorageError>;

/// ストレージエラー
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageError {
    pub message: String,
    pub key: Option<String>,
}

/// ストレージプラグインのトレイト
///
/// R2/S3 等のオブジェクトストレージへの非同期 I/O を抽象化する。
/// `#[async_trait]` により `dyn StoragePlugin` として利用可能。
#[async_trait]
pub trait StoragePlugin: Send + Sync {
    /// オブジェクトをアップロードする
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()>;

    /// オブジェクトをダウンロードする
    async fn get(&self, key: &str) -> StorageResult<Vec<u8>>;

    /// オブジェクトを削除する
    async fn delete(&self, key: &str) -> StorageResult<()>;

    /// プレフィックス配下のキー一覧を返す
    async fn list(&self, prefix: &str) -> StorageResult<Vec<String>>;
}
