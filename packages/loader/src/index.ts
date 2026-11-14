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
const DEFAULT_CONCURRENCY = 5;
const MAX_CONCURRENCY = 10;

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
    concurrency = DEFAULT_CONCURRENCY,
    signal,
  } = options;

  // ── 1. manifest.json 取得（signal があれば中断も伝える）
  const manifest = await fetchManifest(manifestUrl, useCache, false, signal);

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

  // 並列数は 1〜10 にクランプする。1 は従来の逐次取得と等価。
  const workerCount = Math.min(
    MAX_CONCURRENCY,
    Math.max(1, Math.floor(concurrency) || DEFAULT_CONCURRENCY)
  );

  // ── 3〜7. ワーカープールで並列取得
  // JS はシングルスレッドなので、共有カーソル `next` のインクリメントは
  // await の合間にだけ起き、ワーカー同士で同じファイルを取り合うことはない。
  // 進捗の completedFiles は「完了したファイル数」の共有カウンタ。
  let next = 0;
  let completed = 0;

  const loadOne = async (fileKey: string): Promise<void> => {
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
        // exactOptionalPropertyTypes: true のため、undefined の onProgress / signal は渡さない
        ...(onProgress ? { onProgress } : {}),
        ...(signal ? { signal } : {}),
        completedFiles: completed,
        totalFiles,
      });

      // ── 6. Cache API に保存
      if (useCache && strategy.cache) {
        await cachePut(url, data, contentType);
      }
    }

    resultAssets[fileKey] = { url, data, hash, contentType, cached };

    // 進捗: ファイル完了時にも報告
    completed += 1;
    onProgress?.({
      loaded: data.byteLength,
      total: data.byteLength,
      completedFiles: completed,
      totalFiles,
    });
  };

  const worker = async (): Promise<void> => {
    while (next < strategy.files.length) {
      // 次のファイルを取り出す前に中断を確認する（協調的キャンセル）
      signal?.throwIfAborted();
      const fileKey = strategy.files[next]!;
      next += 1;
      await loadOne(fileKey);
    }
  };

  // Promise.all は最初の失敗で reject するので、従来の
  // 「1件でも失敗したら strategy 全体が失敗する」と同じ意味になる。
  await Promise.all(
    Array.from({ length: Math.min(workerCount, totalFiles) }, () => worker())
  );

  const result: StrategyAssetsResult = {
    name,
    assets: resultAssets,
    manifestVersion: manifest.version,
  };

  onComplete?.(result);
  return result;
}
