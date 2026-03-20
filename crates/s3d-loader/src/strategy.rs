//! 配信戦略の宣言型設定モジュール
//!
//! Issue #5 — `assetsStrategy` に対応する構造体群を定義する。
//! CSS / JS / 画像 / フォント / JSON / HTML 断片など、
//! あらゆる静的ファイルの配信戦略を宣言的に制御する。

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────
// ReloadTrigger / ReloadStrategy
// ─────────────────────────────────────────────

/// manifest 再取得のトリガー種別
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReloadTrigger {
    /// manifest の変更を検知したとき
    ManifestChange,
    /// 一定間隔ごと
    Interval,
    /// 明示的な呼び出しのみ
    Manual,
}

/// 再ロード時に差分だけ取得するか全取得するか
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReloadStrategy {
    /// 差分アセットのみ取得
    Diff,
    /// 全アセットを再取得
    Full,
}

// ─────────────────────────────────────────────
// サブ設定構造体
// ─────────────────────────────────────────────

/// 初期表示アセットの設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialConfig {
    /// 初期表示に使うファイルパス一覧（manifest キー）
    pub sources: Vec<String>,
    /// ブラウザキャッシュ（Cache API）に保存するか
    pub cache: bool,
    /// キャッシュにもCDNにもない時の代替ファイルパス
    pub fallback: Option<String>,
}

/// CDN 配信アセットの設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdnStrategyConfig {
    /// CDN 経由で非同期取得するファイルパターン（glob 形式）
    pub files: Vec<String>,
    /// CDN 取得後にキャッシュ有効化するか
    pub cache: bool,
    /// キャッシュ有効期間（例: `"1d"`, `"2h"`）
    pub max_age: Option<String>,
}

/// 再ロードの設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadConfig {
    /// 再ロードトリガー（manifest-change / interval / manual）
    pub trigger: ReloadTrigger,
    /// 差分取得か全取得か
    pub strategy: ReloadStrategy,
    /// `Interval` トリガー時のポーリング間隔（ミリ秒）
    pub interval_ms: Option<u64>,
}

// ─────────────────────────────────────────────
// AssetsStrategyConfig — トップレベル
// ─────────────────────────────────────────────

/// アセット配信戦略の宣言型設定
///
/// TypeScript の `AssetsStrategyConfig` に対応する。
/// ローダー初期化時に渡し、`strategy_assets()` で配信ポリシーを決定する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetsStrategyConfig {
    /// 初期表示アセットの設定
    pub initial: InitialConfig,
    /// CDN 配信アセットの設定
    pub cdn: CdnStrategyConfig,
    /// 再ロードポリシー
    pub reload: ReloadConfig,
}

// ─────────────────────────────────────────────
// strategyAssets の結果型
// ─────────────────────────────────────────────

/// `strategy_assets()` が返す取得済みアセット情報
#[derive(Debug, Clone)]
pub struct StrategyAsset {
    /// manifest キー（例: `"js/main.js"`）
    pub key: String,
    /// CDN URL
    pub url: String,
    /// SHA-256 ハッシュ（hex）
    pub hash: String,
    /// ファイルサイズ（バイト）
    pub size: u64,
    /// レスポンスボディ（bytes）
    pub data: Vec<u8>,
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AssetsStrategyConfig {
        AssetsStrategyConfig {
            initial: InitialConfig {
                sources: vec!["js/main.js".to_string(), "style.css".to_string()],
                cache: true,
                fallback: Some("js/fallback.js".to_string()),
            },
            cdn: CdnStrategyConfig {
                files: vec!["models/**/*.glb".to_string()],
                cache: true,
                max_age: Some("1d".to_string()),
            },
            reload: ReloadConfig {
                trigger: ReloadTrigger::ManifestChange,
                strategy: ReloadStrategy::Diff,
                interval_ms: None,
            },
        }
    }

    #[test]
    fn config_roundtrip_json() {
        let cfg = sample_config();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AssetsStrategyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.initial.sources, cfg.initial.sources);
        assert_eq!(back.cdn.max_age, Some("1d".to_string()));
        assert_eq!(back.reload.trigger, ReloadTrigger::ManifestChange);
        assert_eq!(back.reload.strategy, ReloadStrategy::Diff);
    }

    #[test]
    fn reload_trigger_serde_kebab() {
        let json = serde_json::to_string(&ReloadTrigger::ManifestChange).unwrap();
        assert_eq!(json, r#""manifest-change""#);
        let back: ReloadTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReloadTrigger::ManifestChange);
    }

    #[test]
    fn reload_strategy_serde() {
        assert_eq!(
            serde_json::to_string(&ReloadStrategy::Diff).unwrap(),
            r#""diff""#
        );
        assert_eq!(
            serde_json::to_string(&ReloadStrategy::Full).unwrap(),
            r#""full""#
        );
    }
}
