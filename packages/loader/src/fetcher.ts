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
  /** 呼び出し側からの中断シグナル（省略可） */
  signal?: AbortSignal;
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
      // 中断は再試行しない（AbortError はそのまま投げ直す）
      if (err instanceof Error && err.name === "AbortError") {
        throw err;
      }
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
  // タイムアウトと外部シグナルを1本のコントローラに合成する。
  // AbortSignal.any() は使わない（古いブラウザで未対応のため）。リスナー方式なら
  // どのブラウザでも動く。既に abort 済みなら fetch 前に即座に失敗する。
  if (opts.signal?.aborted) {
    opts.signal.throwIfAborted();
  }
  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), opts.timeout);
  const onExternalAbort = () => controller.abort();
  opts.signal?.addEventListener("abort", onExternalAbort, { once: true });

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
    opts.signal?.removeEventListener("abort", onExternalAbort);
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

  try {
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
  } finally {
    // 読み取り中に中断されたらストリームを解放する（でないと残る）
    if (opts.signal?.aborted) {
      await reader.cancel().catch(() => {});
    }
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
