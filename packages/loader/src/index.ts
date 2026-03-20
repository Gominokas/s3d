/**
 * @statics-lead/loader — エントリポイント
 *
 * strategyAssets(name, options?) をエクスポートする。
 *
 * ## 処理フロー
 * 1. manifest.json を fetch（キャッシュがあればキャッシュから）
 * 2. manifest.strategies[name] を参照して対象ファイル一覧を取得
 * 3. 各ファイルの CDN URL を manifest.assets から解決
 * 4. Cache API にキャッシュがあればそこから返す（hash 変更時は evict して再取得）
 * 5. CDN URL から fetch（タイムアウト・リトライ・進捗コールバック）
 * 6. Cache API に保存
 * 7. StrategyAssetsResult を返す
 */

import { fetchManifest } from "./manifest.js";
import { cachePut, cacheGet, cacheEvict } from "./cache.js";
import { fetchAsset } from "./fetcher.js";
import type {
  FetchOptions,
  StrategyAssetsResult,
  AssetData,
} from "./types.js";

export type {
  FetchOptions,
  StrategyAssetsResult,
  AssetData,
  ManifestEntry,
  StrategyEntry,
  StrategyReload,
  DeployManifest,
  ProgressEvent,
} from "./types.js";

const DEFAULT_MANIFEST_URL = "/manifest.json";
const DEFAULT_TIMEOUT = 30_000;
const DEFAULT_RETRIES = 2;

/**
 * 指定した strategy 名のアセットを非同期で取得する。
 *
 * @param name    - strategy 名（manifest.strategies のキー）
 * @param options - 取得オプション
 * @returns       StrategyAssetsResult
 *
 * @example
 * ```ts
 * const result = await strategyAssets("sushi");
 * // result.assets["assets/sushi.glb"].data -> ArrayBuffer
 * // result.assets["assets/sushi.glb"].cached -> boolean
 * ```
 */
export async function strategyAssets(
  name: string,
  options: FetchOptions = {}
): Promise<StrategyAssetsResult> {
  const {
    manifestUrl = DEFAULT_MANIFEST_URL,
    cache: useCache = true,
    onProgress,
    onComplete,
    timeout = DEFAULT_TIMEOUT,
    retries = DEFAULT_RETRIES,
  } = options;

  // ── 1. manifest.json 取得
  const manifest = await fetchManifest(manifestUrl, useCache);

  // ── 2. strategy エントリを確認
  const strategy = manifest.strategies[name];
  if (!strategy) {
    throw new Error(
      `strategy "${name}" が manifest.json に存在しません。` +
        ` 利用可能な strategy: ${Object.keys(manifest.strategies).join(", ") || "(なし)"}`
    );
  }

  const totalFiles = strategy.files.length;
  const resultAssets: Record<string, AssetData> = {};

  // ── 3〜7. ファイルごとに取得
  for (let i = 0; i < strategy.files.length; i++) {
    const fileKey = strategy.files[i]!;

    // manifest.assets からエントリを解決
    const entry = manifest.assets[fileKey];
    if (!entry) {
      throw new Error(
        `manifest.assets に "${fileKey}" が存在しません。` +
          `s3d build を再実行してください。`
      );
    }

    const { url, hash, contentType } = entry;

    // ── 4. Cache API からヒット確認
    let data: ArrayBuffer | null = null;
    let cached = false;

    if (useCache && strategy.cache) {
      const cachedData = await cacheGet(url);
      if (cachedData !== null) {
        data = cachedData;
        cached = true;
      }
    }

    // ── 5. キャッシュミス → CDN から fetch
    if (data === null) {
      // ハッシュ変更時に古いキャッシュを evict（旧 URL が残っていた場合）
      if (useCache && strategy.cache) {
        await cacheEvict(url);
      }

      data = await fetchAsset(url, {
        timeout,
        retries,
        onProgress,
        completedFiles: i,
        totalFiles,
      });

      // ── 6. Cache API に保存
      if (useCache && strategy.cache) {
        await cachePut(url, data, contentType);
      }
    }

    resultAssets[fileKey] = { url, data, hash, contentType, cached };

    // 進捗: ファイル完了時にも報告
    onProgress?.({
      loaded: data.byteLength,
      total: data.byteLength,
      completedFiles: i + 1,
      totalFiles,
    });
  }

  const result: StrategyAssetsResult = {
    name,
    assets: resultAssets,
    manifestVersion: manifest.version,
  };

  onComplete?.(result);
  return result;
}
