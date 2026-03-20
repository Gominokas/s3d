/**
 * manifest.ts のユニットテスト
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { fetchManifest, evictManifestCache } from "../src/manifest.js";
import type { DeployManifest } from "../src/types.js";

// ─────────────────────────────────────────────────────────────
// サンプルマニフェスト
// ─────────────────────────────────────────────────────────────

const sampleManifest: DeployManifest = {
  schemaVersion: 1,
  version: "1.0.0",
  buildTime: "2026-03-20T00:00:00Z",
  assets: {
    "assets/sushi.glb": {
      url: "https://cdn.example.com/assets/sushi.abcd1234.glb",
      size: 1024,
      hash: "abcd1234",
      contentType: "model/gltf-binary",
    },
  },
  strategies: {
    sushi: {
      files: ["assets/sushi.glb"],
      initial: false,
      cache: true,
      maxAge: "7d",
    },
  },
};

// ─────────────────────────────────────────────────────────────
// fetch モック
// ─────────────────────────────────────────────────────────────

function mockFetchOk(manifest: DeployManifest) {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      statusText: "OK",
      json: async () => manifest,
    })
  );
}

function mockFetchError(status: number) {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: false,
      status,
      statusText: "Not Found",
    })
  );
}

// ─────────────────────────────────────────────────────────────
// Cache API モック（manifest キャッシュ用）
// ─────────────────────────────────────────────────────────────

function makeCachesMock(initialData?: DeployManifest) {
  const store = new Map<string, Response>();
  if (initialData) {
    store.set(
      "/manifest.json",
      new Response(JSON.stringify(initialData), {
        headers: { "Content-Type": "application/json" },
      })
    );
  }

  const cacheMock = {
    put: vi.fn(async (url: string, res: Response) => { store.set(url, res.clone()); }),
    match: vi.fn(async (url: string) => store.get(url)?.clone()),
    delete: vi.fn(async (url: string) => { store.delete(url); }),
  };

  return {
    open: vi.fn(async () => cacheMock),
    delete: vi.fn(async () => { store.clear(); }),
    cacheMock,
  };
}

beforeEach(() => {
  vi.restoreAllMocks();
});

// ─────────────────────────────────────────────────────────────
// テスト
// ─────────────────────────────────────────────────────────────

describe("fetchManifest — ネットワーク取得", () => {
  it("正常にパースされる", async () => {
    mockFetchOk(sampleManifest);
    vi.stubGlobal("caches", undefined);

    const manifest = await fetchManifest("/manifest.json", false);
    expect(manifest.version).toBe("1.0.0");
    expect(manifest.assets["assets/sushi.glb"]).toBeDefined();
    expect(manifest.strategies["sushi"]).toBeDefined();
  });

  it("HTTP エラー時にエラーをスローする", async () => {
    mockFetchError(404);
    vi.stubGlobal("caches", undefined);

    await expect(fetchManifest("/manifest.json", false)).rejects.toThrow("404");
  });
});

describe("fetchManifest — Cache API 使用", () => {
  it("キャッシュがあればネットワーク fetch をスキップする", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("caches", makeCachesMock(sampleManifest));

    const manifest = await fetchManifest("/manifest.json", true);
    expect(manifest.version).toBe("1.0.0");
    // ネットワーク fetch は呼ばれない
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("キャッシュがなければネットワーク fetch してキャッシュに保存する", async () => {
    mockFetchOk(sampleManifest);
    const cachesMock = makeCachesMock(); // 空キャッシュ
    vi.stubGlobal("caches", cachesMock);

    const manifest = await fetchManifest("/manifest.json", true);
    expect(manifest.version).toBe("1.0.0");
    expect(vi.mocked(fetch)).toHaveBeenCalledWith("/manifest.json");
    expect(cachesMock.cacheMock.put).toHaveBeenCalled();
  });

  it("forceRefresh=true ならキャッシュを無視してネットワーク取得する", async () => {
    mockFetchOk(sampleManifest);
    vi.stubGlobal("caches", makeCachesMock(sampleManifest));

    const manifest = await fetchManifest("/manifest.json", true, true);
    expect(manifest.version).toBe("1.0.0");
    expect(vi.mocked(fetch)).toHaveBeenCalled();
  });
});

describe("evictManifestCache", () => {
  it("Cache API が利用可能なとき manifest キャッシュを削除する", async () => {
    const cachesMock = makeCachesMock(sampleManifest);
    vi.stubGlobal("caches", cachesMock);

    await evictManifestCache("/manifest.json");
    expect(cachesMock.cacheMock.delete).toHaveBeenCalledWith("/manifest.json");
  });

  it("Cache API が非サポートでもエラーにならない", async () => {
    vi.stubGlobal("caches", undefined);
    await expect(evictManifestCache("/manifest.json")).resolves.toBeUndefined();
  });
});
