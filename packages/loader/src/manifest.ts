/**
 * manifest.json の fetch とパース
 *
 * - ブラウザの Cache API（caches）でマニフェスト自体をキャッシュする
 * - `?v={buildTime}` をキャッシュキーに付加してバージョン衝突を防ぐ
 * - buildTime が変わったら自動的に古いキャッシュを evict して新しいものを格納する
 */

import type { DeployManifest } from "./types.js";

const MANIFEST_CACHE_NAME = "s3d-manifest-v1";

/**
 * manifest.json を取得してパースする。
 *
 * Cache API が利用可能であれば、前回取得時のレスポンスを返す。
 * ただし `forceRefresh = true` の場合はネットワークから再取得する。
 *
 * キャッシュキーは `{manifestUrl}?v={buildTime}` — buildTime が変わると
 * 別キーとして扱われ、古いエントリは自動的に削除される。
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
    // バージョン付きキーも含めて全エントリを列挙して削除
    const keys = await cache.keys();
    const prefix = manifestUrl.split("?")[0]!;
    for (const request of keys) {
      if (request.url.split("?")[0] === prefix) {
        await cache.delete(request);
      }
    }
  } catch {
    // キャッシュ操作の失敗は無視
  }
}

// ─────────────────────────────────────────────────────────────
// 内部ヘルパー
// ─────────────────────────────────────────────────────────────

/**
 * `{manifestUrl}?v={buildTime}` のキャッシュキーを生成する。
 *
 * buildTime は manifest.json を一度取得してから判明するため、
 * キャッシュ保存時に versioned key を使い、
 * キャッシュ参照時は最新 buildTime をバージョンポインタから読む。
 */
function versionedUrl(baseUrl: string, buildTime: string): string {
  return `${baseUrl}?v=${encodeURIComponent(buildTime)}`;
}

/** バージョンポインタ（plain URL → buildTime）を返す専用キーのプレフィックス */
const VERSION_POINTER_PREFIX = "s3d:ptr:";

async function tryGetFromCache(
  manifestUrl: string
): Promise<DeployManifest | null> {
  try {
    const cache = await caches.open(MANIFEST_CACHE_NAME);

    // バージョンポインタ（buildTime）を取得
    const ptrKey = VERSION_POINTER_PREFIX + manifestUrl;
    const ptrResponse = await cache.match(ptrKey);
    if (!ptrResponse) return null;

    const buildTime = await ptrResponse.text();
    if (!buildTime) return null;

    // バージョン付きキーでマニフェスト取得
    const versioned = versionedUrl(manifestUrl, buildTime);
    const response = await cache.match(versioned);
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
    const buildTime = manifest.buildTime;

    // 1. バージョン付きキーでマニフェスト本体を保存
    const versioned = versionedUrl(manifestUrl, buildTime);
    const body = JSON.stringify(manifest);
    const manifestResponse = new Response(body, {
      headers: { "Content-Type": "application/json" },
    });
    await cache.put(versioned, manifestResponse);

    // 2. バージョンポインタを更新（plain URL → buildTime）
    //    古いバージョン付きキーは次のアクセス時に自然に参照されなくなる
    const ptrKey = VERSION_POINTER_PREFIX + manifestUrl;
    const ptrResponse = new Response(buildTime, {
      headers: { "Content-Type": "text/plain" },
    });
    await cache.put(ptrKey, ptrResponse);

    // 3. 古いバージョン付きキーを削除（evict）
    const keys = await cache.keys();
    const base = manifestUrl.split("?")[0]!;
    for (const request of keys) {
      const url = request.url;
      // バージョンポインタキーはスキップ
      if (url.includes(VERSION_POINTER_PREFIX)) continue;
      // base URL が一致し、かつ現バージョン以外のキーを削除
      if (url.split("?")[0] === base && url !== versioned) {
        await cache.delete(request);
      }
    }
  } catch {
    // キャッシュ書き込み失敗は無視（フォールバックでネットワーク取得）
  }
}
