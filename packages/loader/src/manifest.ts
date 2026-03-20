/**
 * manifest.json の fetch とパース
 *
 * - ブラウザの Cache API（caches）でマニフェスト自体をキャッシュする
 * - ハッシュ（buildTime + version）が変わったら自動的に更新する
 */

import type { DeployManifest } from "./types.js";

const MANIFEST_CACHE_NAME = "s3d-manifest-v1";

/**
 * manifest.json を取得してパースする。
 *
 * Cache API が利用可能であれば、前回取得時のレスポンスを返す。
 * ただし `forceRefresh = true` の場合はネットワークから再取得する。
 */
export async function fetchManifest(
  manifestUrl: string,
  useCache: boolean,
  forceRefresh = false
): Promise<DeployManifest> {
  if (useCache && !forceRefresh && typeof caches !== "undefined") {
    const cached = await tryGetFromCache(manifestUrl);
    if (cached !== null) {
      return cached;
    }
  }

  const manifest = await fetchManifestFromNetwork(manifestUrl);

  if (useCache && typeof caches !== "undefined") {
    await putManifestToCache(manifestUrl, manifest);
  }

  return manifest;
}

/**
 * manifest.json のキャッシュを削除する（バージョン変更時に呼ぶ）。
 */
export async function evictManifestCache(manifestUrl: string): Promise<void> {
  if (typeof caches === "undefined") return;
  try {
    const cache = await caches.open(MANIFEST_CACHE_NAME);
    await cache.delete(manifestUrl);
  } catch {
    // キャッシュ操作の失敗は無視
  }
}

// ─────────────────────────────────────────────────────────────
// 内部ヘルパー
// ─────────────────────────────────────────────────────────────

async function tryGetFromCache(
  manifestUrl: string
): Promise<DeployManifest | null> {
  try {
    const cache = await caches.open(MANIFEST_CACHE_NAME);
    const response = await cache.match(manifestUrl);
    if (!response) return null;
    const json = await response.json();
    return json as DeployManifest;
  } catch {
    return null;
  }
}

async function fetchManifestFromNetwork(
  manifestUrl: string
): Promise<DeployManifest> {
  const response = await fetch(manifestUrl);
  if (!response.ok) {
    throw new Error(
      `manifest.json の取得に失敗しました: ${response.status} ${response.statusText} (${manifestUrl})`
    );
  }
  const json = await response.json();
  return json as DeployManifest;
}

async function putManifestToCache(
  manifestUrl: string,
  manifest: DeployManifest
): Promise<void> {
  try {
    const cache = await caches.open(MANIFEST_CACHE_NAME);
    const body = JSON.stringify(manifest);
    const response = new Response(body, {
      headers: { "Content-Type": "application/json" },
    });
    await cache.put(manifestUrl, response);
  } catch {
    // キャッシュ書き込み失敗は無視（フォールバックでネットワーク取得）
  }
}
