/**
 * Cache API ラッパー
 *
 * アセット本体のキャッシュ管理を担当する。
 * - put: ArrayBuffer をキャッシュに保存
 * - get: キャッシュからデータを取得（ない場合は null）
 * - has: キャッシュ存在確認
 * - evict: 特定 URL のキャッシュを削除
 * - evictAll: キャッシュ全削除
 *
 * ブラウザが Cache API をサポートしていない場合はすべてノーオペレーションになる。
 */

const ASSET_CACHE_NAME = "s3d-assets-v1";

/**
 * アセットデータを Cache API に保存する。
 *
 * @param url       - CDN URL（キャッシュキー）
 * @param data      - アセットデータ（ArrayBuffer）
 * @param contentType - MIME タイプ
 */
export async function cachePut(
  url: string,
  data: ArrayBuffer,
  contentType: string
): Promise<void> {
  if (typeof caches === "undefined") return;
  try {
    const cache = await caches.open(ASSET_CACHE_NAME);
    const response = new Response(data, {
      headers: {
        "Content-Type": contentType,
        "X-S3d-Cached-At": Date.now().toString(),
      },
    });
    await cache.put(url, response);
  } catch {
    // キャッシュ書き込み失敗は無視
  }
}

/**
 * Cache API からアセットデータを取得する。
 *
 * @returns ArrayBuffer、またはキャッシュミスの場合は null
 */
export async function cacheGet(url: string): Promise<ArrayBuffer | null> {
  if (typeof caches === "undefined") return null;
  try {
    const cache = await caches.open(ASSET_CACHE_NAME);
    const response = await cache.match(url);
    if (!response) return null;
    return await response.arrayBuffer();
  } catch {
    return null;
  }
}

/**
 * 指定 URL のキャッシュが存在するか確認する。
 */
export async function cacheHas(url: string): Promise<boolean> {
  if (typeof caches === "undefined") return false;
  try {
    const cache = await caches.open(ASSET_CACHE_NAME);
    const response = await cache.match(url);
    return response !== undefined;
  } catch {
    return false;
  }
}

/**
 * 指定 URL のキャッシュを削除する（ハッシュ変更時に呼ぶ）。
 */
export async function cacheEvict(url: string): Promise<void> {
  if (typeof caches === "undefined") return;
  try {
    const cache = await caches.open(ASSET_CACHE_NAME);
    await cache.delete(url);
  } catch {
    // 無視
  }
}

/**
 * キャッシュ全体を削除する。
 */
export async function cacheEvictAll(): Promise<void> {
  if (typeof caches === "undefined") return;
  try {
    await caches.delete(ASSET_CACHE_NAME);
  } catch {
    // 無視
  }
}
