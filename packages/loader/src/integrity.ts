/**
 * 完全性検証モジュール（#54）
 *
 * manifest.json の `hash` は SHA-256 hex の先頭 8 文字だけ
 * （`crates/s3d-deploy/src/hash.rs`、`DEFAULT_HASH_LENGTH = 8`）。
 * ここではその 8 文字フィンガープリントで照合する暫定検証を行う。
 *
 * - 目的は破損検出（転送・保存中のデータ化けの検出）。
 * - 8 文字（32 bit）の照合は衝突確率 2^-32 で、セキュリティ保証にはならない。
 *   ハッシュは署名でもない（manifest とファイルが同じ経路で改ざんされれば通る）。
 * - 完全な検証（64 文字の full hash 照合）には manifest への full hash 追加が
 *   必要で、#49 の範囲。その対応までの暫定版。
 *
 * 検証は `crypto.subtle.digest("SHA-256", ...)` で行う。HTTPS など
 * secure context が前提。`crypto.subtle` が無い環境では検証を要求した
 * 呼び出しがエラーになる（検証なしで通すことはしない）。
 */

/** ArrayBuffer の SHA-256 を 64 文字の hex 文字列で返す。 */
export async function computeSha256Hex(data: ArrayBuffer): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new Error(
      "完全性検証には crypto.subtle が必要です（HTTPS などの secure context で利用してください）。"
    );
  }
  const digest = await subtle.digest("SHA-256", data);
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** フィンガープリント不一致のエラー。キャッシュ側の不一致は evict＋再取得で回復できるので、呼び出し側で区別できるよう専用クラスにする。 */
export class IntegrityMismatchError extends Error {
  constructor(fileKey: string, expectedHash: string, actual: string) {
    super(
      `完全性検証に失敗しました: "${fileKey}" のハッシュが manifest と一致しません。` +
        ` 期待値: ${expectedHash}、実際: ${actual}。` +
          `s3d build を再実行するか、配信経路を確認してください。`
    );
    this.name = "IntegrityMismatchError";
  }
}

/**
 * ファイルデータの SHA-256 が manifest の hash（先頭 N 文字）と
 * 一致するか確認する。一致しなければ IntegrityMismatchError をスローする。
 *
 * @param data - 検証するファイルデータ
 * @param expectedHash - manifest の hash（8 文字のフィンガープリント）。
 *   将来 full hash（64 文字）が入っても、その長さ分だけ照合する。
 * @param fileKey - エラーメッセージ用のファイルキー
 */
export async function verifyFingerprint(
  data: ArrayBuffer,
  expectedHash: string,
  fileKey: string
): Promise<void> {
  const actualHex = await computeSha256Hex(data);
  const actual = actualHex.slice(0, expectedHash.length);
  if (actual !== expectedHash) {
    throw new IntegrityMismatchError(fileKey, expectedHash, actual);
  }
}
