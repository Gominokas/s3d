/**
 * アセット fetch モジュール
 *
 * - タイムアウト（AbortController）
 * - リトライ（指数バックオフ）
 * - 進捗コールバック（ReadableStream を使って読み取り）
 */

import type { ProgressEvent } from "./types.js";

export interface FetchAssetOptions {
  timeout: number;
  retries: number;
  onProgress?: (event: ProgressEvent) => void;
  /** 進捗計算用: 全体の完了ファイル数 / 全体ファイル数 */
  completedFiles: number;
  totalFiles: number;
}

/**
 * 1 ファイルを CDN URL から取得して ArrayBuffer で返す。
 *
 * タイムアウト・リトライ・進捗コールバックをサポートする。
 */
export async function fetchAsset(
  url: string,
  opts: FetchAssetOptions
): Promise<ArrayBuffer> {
  let lastError: unknown;

  for (let attempt = 0; attempt <= opts.retries; attempt++) {
    if (attempt > 0) {
      // 指数バックオフ: 500ms, 1000ms, ...
      await sleep(500 * Math.pow(2, attempt - 1));
    }
    try {
      return await fetchWithTimeout(url, opts);
    } catch (err) {
      lastError = err;
    }
  }

  throw new Error(
    `アセット取得に失敗しました (${opts.retries + 1} 回試行): ${url}\n${String(lastError)}`
  );
}

// ─────────────────────────────────────────────────────────────
// 内部ヘルパー
// ─────────────────────────────────────────────────────────────

async function fetchWithTimeout(
  url: string,
  opts: FetchAssetOptions
): Promise<ArrayBuffer> {
  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), opts.timeout);

  try {
    const response = await fetch(url, { signal: controller.signal });

    if (!response.ok) {
      throw new Error(
        `HTTP ${response.status} ${response.statusText}: ${url}`
      );
    }

    if (opts.onProgress && response.body) {
      return await readWithProgress(response, opts);
    }

    return await response.arrayBuffer();
  } finally {
    clearTimeout(timerId);
  }
}

/**
 * ReadableStream を逐次読み取りながら進捗を報告する。
 */
async function readWithProgress(
  response: Response,
  opts: FetchAssetOptions
): Promise<ArrayBuffer> {
  const contentLength = Number(response.headers.get("Content-Length") ?? "0");
  const reader = response.body!.getReader();
  const chunks: Uint8Array[] = [];
  let loaded = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    chunks.push(value);
    loaded += value.byteLength;

    opts.onProgress?.({
      loaded,
      total: contentLength,
      completedFiles: opts.completedFiles,
      totalFiles: opts.totalFiles,
    });
  }

  // チャンクを結合して ArrayBuffer を返す
  const totalBytes = chunks.reduce((sum, c) => sum + c.byteLength, 0);
  const result = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result.buffer;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
