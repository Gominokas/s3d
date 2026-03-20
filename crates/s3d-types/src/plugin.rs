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
/// オブジェクトストレージへの put / get / delete / list を抽象化する。
pub trait StoragePlugin {
    /// オブジェクトをアップロードする
    fn put(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()>;

    /// オブジェクトをダウンロードする
    fn get(&self, key: &str) -> StorageResult<Vec<u8>>;

    /// オブジェクトを削除する
    fn delete(&self, key: &str) -> StorageResult<()>;

    /// プレフィックス配下のキー一覧を返す
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>>;
}
