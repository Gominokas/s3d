//! s3d-deploy — アセット収集・ハッシュ化・マニフェスト生成・差分計算クレート
//!
//! TypeScript パッケージ `bk/static3d/packages/deploy/src/build/` に対応する。
//!
//! ## 処理フロー
//! ```text
//! collect() → hash_assets() → build_manifest() → diff_manifests()
//! ```

pub mod collect;
pub mod diff;
pub mod hash;
pub mod manifest;
