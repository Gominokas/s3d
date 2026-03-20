/**
 * cache.ts のユニットテスト
 *
 * happy-dom は Cache API をサポートしていないため、
 * グローバルの caches を vi.stubGlobal でモックする。
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { cachePut, cacheGet, cacheHas, cacheEvict, cacheEvictAll } from "../src/cache.js";

// ─────────────────────────────────────────────────────────────
// Cache API モック
// ─────────────────────────────────────────────────────────────

function makeCacheStoreMock() {
  const store = new Map<string, Response>();

  const cacheMock = {
    put: vi.fn(async (url: string, res: Response) => {
      store.set(url, res);
    }),
    match: vi.fn(async (url: string) => store.get(url)),
    delete: vi.fn(async (url: string) => store.delete(url)),
  };

  const cachesMock = {
    open: vi.fn(async () => cacheMock),
    delete: vi.fn(async () => { store.clear(); }),
  };

  return { store, cacheMock, cachesMock };
}

let mocks: ReturnType<typeof makeCacheStoreMock>;

beforeEach(() => {
  mocks = makeCacheStoreMock();
  vi.stubGlobal("caches", mocks.cachesMock);
});

// ─────────────────────────────────────────────────────────────
// テスト
// ─────────────────────────────────────────────────────────────

describe("cachePut / cacheGet", () => {
  it("ArrayBuffer を保存して取得できる", async () => {
    const data = new TextEncoder().encode("hello").buffer;
    await cachePut("https://cdn.example.com/file.bin", data, "application/octet-stream");
    const result = await cacheGet("https://cdn.example.com/file.bin");
    expect(result).not.toBeNull();
    expect(new TextDecoder().decode(result!)).toBe("hello");
  });

  it("存在しない URL は null を返す", async () => {
    const result = await cacheGet("https://cdn.example.com/missing.bin");
    expect(result).toBeNull();
  });
});

describe("cacheHas", () => {
  it("存在するキャッシュは true を返す", async () => {
    const data = new TextEncoder().encode("x").buffer;
    await cachePut("https://cdn.example.com/a.bin", data, "application/octet-stream");
    expect(await cacheHas("https://cdn.example.com/a.bin")).toBe(true);
  });

  it("存在しないキャッシュは false を返す", async () => {
    expect(await cacheHas("https://cdn.example.com/none.bin")).toBe(false);
  });
});

describe("cacheEvict", () => {
  it("指定 URL のキャッシュを削除する", async () => {
    const data = new TextEncoder().encode("x").buffer;
    await cachePut("https://cdn.example.com/del.bin", data, "application/octet-stream");
    await cacheEvict("https://cdn.example.com/del.bin");
    expect(await cacheGet("https://cdn.example.com/del.bin")).toBeNull();
  });
});

describe("cacheEvictAll", () => {
  it("全キャッシュを削除する", async () => {
    await cacheEvictAll();
    expect(mocks.cachesMock.delete).toHaveBeenCalledWith("s3d-assets-v1");
  });
});

describe("Cache API 非サポート環境", () => {
  it("caches が undefined でもエラーにならない", async () => {
    vi.stubGlobal("caches", undefined);
    const data = new TextEncoder().encode("x").buffer;
    await expect(cachePut("https://cdn.example.com/x.bin", data, "application/octet-stream")).resolves.toBeUndefined();
    await expect(cacheGet("https://cdn.example.com/x.bin")).resolves.toBeNull();
    await expect(cacheHas("https://cdn.example.com/x.bin")).resolves.toBe(false);
    await expect(cacheEvict("https://cdn.example.com/x.bin")).resolves.toBeUndefined();
    await expect(cacheEvictAll()).resolves.toBeUndefined();
  });
});
