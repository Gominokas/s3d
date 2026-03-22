/**
 * @statics-lead/loader — 型定義
 *
 * manifest.json の構造と strategyAssets() の API 型を定義する。
 * Rust 側の s3d-types/src/manifest.rs の構造体に対応する。
 */

// ─────────────────────────────────────────────────────────────
// Manifest 型
// ─────────────────────────────────────────────────────────────

/** manifest.json の各アセットエントリ（Rust: AssetEntry） */
export interface ManifestEntry {
  /** CDN 上の完全 URL */
  url: string;
  /** ファイルサイズ（バイト） */
  size: number;
  /** SHA-256 ハッシュの先頭 8 文字 */
  hash: string;
  /** MIME タイプ */
  contentType: string;
  /** glTF 依存アセットのハッシュ付きキー一覧（optional） */
  dependencies?: string[];
}

/** strategy.json の reload セクション（Rust: StrategyReload） */
export interface StrategyReload {
  trigger: string;
  strategy: string;
}

/** manifest.json の strategies[name] エントリ（Rust: StrategyEntry） */
export interface StrategyEntry {
  /** CDN 経由で配信するファイルのキー一覧（manifest.assets のキーと一致） */
  files: string[];
  /**
   * 初期表示ファイルのパス（ハッシュなしでコピーされる）。
   * 省略時は初期表示なし。例: "assets/placeholder.png"
   */
  initial?: string;
  /** Cache API を使うか */
  cache: boolean;
  /** キャッシュ最大有効期間（例: "7d"） */
  maxAge?: string;
  /** リロード設定 */
  reload?: StrategyReload;
}

/** manifest.json 全体（Rust: DeployManifest） */
export interface DeployManifest {
  schemaVersion: number;
  version: string;
  buildTime: string;
  /** キー: 元のファイルパス（例: "assets/sushi.glb"） */
  assets: Record<string, ManifestEntry>;
  /** キー: strategy 名（例: "sushi"） */
  strategies: Record<string, StrategyEntry>;
}

// ─────────────────────────────────────────────────────────────
// strategyAssets() API 型
// ─────────────────────────────────────────────────────────────

/** 進捗コールバックの引数 */
export interface ProgressEvent {
  /** 取得済みバイト数 */
  loaded: number;
  /** 合計バイト数（不明な場合は 0） */
  total: number;
  /** 完了済みファイル数 */
  completedFiles: number;
  /** 対象ファイル数 */
  totalFiles: number;
}

/** strategyAssets() のオプション */
export interface FetchOptions {
  /**
   * manifest.json を取得する URL。
   * デフォルト: "/manifest.json"
   */
  manifestUrl?: string;
  /**
   * Cache API を使うか。
   * デフォルト: true（ブラウザが Cache API をサポートしている場合）
   */
  cache?: boolean;
  /**
   * 進捗コールバック
   */
  onProgress?: (event: ProgressEvent) => void;
  /**
   * 全ファイル取得完了時のコールバック
   */
  onComplete?: (result: StrategyAssetsResult) => void;
  /**
   * 1ファイルあたりのタイムアウト（ms）。デフォルト: 30000
   */
  timeout?: number;
  /**
   * リトライ回数。デフォルト: 2
   */
  retries?: number;
}

/** strategyAssets() が返す各ファイルのデータ */
export interface AssetData {
  /** CDN URL */
  url: string;
  /** ファイルデータ（ArrayBuffer） */
  data: ArrayBuffer;
  /** SHA-256 ハッシュ */
  hash: string;
  /** MIME タイプ */
  contentType: string;
  /** Cache API からヒットしたか */
  cached: boolean;
}

/** strategyAssets() の戻り値 */
export interface StrategyAssetsResult {
  /** strategy 名 */
  name: string;
  /** キー: ファイルパス → AssetData */
  assets: Record<string, AssetData>;
  /** manifest のバージョン文字列 */
  manifestVersion: string;
}
