/**
 * strategyAssets() の統合テスト
 *
 * fetch / caches をモックして処理フロー全体を検証する。
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { strategyAssets } from "../src/index.js";
import type { DeployManifest } from "../src/types.js";

// ─────────────────────────────────────────────────────────────
// テスト用マニフェスト
// ─────────────────────────────────────────────────────────────

const glbData = new Uint8Array([0x67, 0x6c, 0x54, 0x46]).buffer; // "glTF"

const sampleManifest: DeployManifest = {
  schemaVersion: 1,
  version: "1.0.0",
  buildTime: "2026-03-20T00:00:00Z",
  assets: {
    "assets/sushi.glb": {
      url: "/assets/sushi.abcd1234.glb",
      size: 4,
      hash: "abcd1234",
      contentType: "model/gltf-binary",
    },
    "assets/gari.glb": {
      url: "/assets/gari.efgh5678.glb",
      size: 4,
      hash: "efgh5678",
      contentType: "model/gltf-binary",
    },
  },
  strategies: {
    sushi: {
      files: ["assets/sushi.glb"],
      initial: false,
      cache: true,
      maxAge: "7d",
      reload: { trigger: "manifest-change", strategy: "diff" },
    },
    combo: {
      files: ["assets/sushi.glb", "assets/gari.glb"],
      initial: false,
      cache: true,
    },
  },
};

// ─────────────────────────────────────────────────────────────
// モックヘルパー
// ─────────────────────────────────────────────────────────────

/** manifest と アセットの fetch をモックする */
function mockFetch(manifest: DeployManifest, assetData: ArrayBuffer = glbData) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      if ((url as string).endsWith(".json")) {
        return {
          ok: true,
          status: 200,
          json: async () => manifest,
          body: null,
        };
      }
      return {
        ok: true,
        status: 200,
        headers: new Headers({ "Content-Length": String(assetData.byteLength) }),
        arrayBuffer: async () => assetData,
        body: null,
      };
    })
  );
}

/** Cache API をモックする（空または初期データあり） */
function mockCaches(initialAssets: Map<string, ArrayBuffer> = new Map()) {
  const assetStore = new Map<string, ArrayBuffer>(initialAssets);
  const manifestStore = new Map<string, string>();

  const cacheMockAssets = {
    put: vi.fn(async (url: string, res: Response) => {
      assetStore.set(url, await res.arrayBuffer());
    }),
    match: vi.fn(async (url: string) => {
      const data = assetStore.get(url);
      if (!data) return undefined;
      return new Response(data, { headers: { "Content-Type": "application/octet-stream" } });
    }),
    delete: vi.fn(async (url: string) => { assetStore.delete(url); }),
  };

  const cacheMockManifest = {
    put: vi.fn(async (url: string, res: Response) => {
      manifestStore.set(url, await res.text());
    }),
    match: vi.fn(async () => undefined), // manifest は常にキャッシュミス
    delete: vi.fn(),
  };

  vi.stubGlobal("caches", {
    open: vi.fn(async (name: string) => {
      if (name === "s3d-assets-v1") return cacheMockAssets;
      return cacheMockManifest;
    }),
    delete: vi.fn(),
    cacheMockAssets,
  });

  return { assetStore, cacheMockAssets };
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.stubGlobal("caches", undefined); // デフォルト: Cache API なし
});

// ─────────────────────────────────────────────────────────────
// テスト
// ─────────────────────────────────────────────────────────────

describe("strategyAssets — 基本動作", () => {
  it("strategy 名で指定したファイルを取得して返す", async () => {
    mockFetch(sampleManifest);

    const result = await strategyAssets("sushi", { cache: false });

    expect(result.name).toBe("sushi");
    expect(result.manifestVersion).toBe("1.0.0");
    expect(result.assets["assets/sushi.glb"]).toBeDefined();

    const asset = result.assets["assets/sushi.glb"]!;
    expect(asset.url).toBe("/assets/sushi.abcd1234.glb");
    expect(asset.hash).toBe("abcd1234");
    expect(asset.contentType).toBe("model/gltf-binary");
    expect(asset.cached).toBe(false);
    expect(asset.data.byteLength).toBe(4);
  });

  it("複数ファイルの strategy を取得できる", async () => {
    mockFetch(sampleManifest);

    const result = await strategyAssets("combo", { cache: false });

    expect(Object.keys(result.assets)).toHaveLength(2);
    expect(result.assets["assets/sushi.glb"]).toBeDefined();
    expect(result.assets["assets/gari.glb"]).toBeDefined();
  });

  it("存在しない strategy 名はエラーをスローする", async () => {
    mockFetch(sampleManifest);

    await expect(strategyAssets("nonexistent", { cache: false })).rejects.toThrow(
      '"nonexistent" が manifest.json に存在しません'
    );
  });
});

describe("strategyAssets — キャッシュ動作", () => {
  it("Cache API にヒットすれば fetch を呼ばない", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      // manifest だけ返す（アセット fetch は呼ばれないはず）
      if ((url as string).endsWith(".json")) {
        return { ok: true, status: 200, json: async () => sampleManifest, body: null };
      }
      throw new Error("アセット fetch が呼ばれてはならない");
    });
    vi.stubGlobal("fetch", fetchMock);

    // アセットをあらかじめキャッシュに入れておく
    const assetUrl = "/assets/sushi.abcd1234.glb";
    const { cacheMockAssets } = mockCaches(new Map([[assetUrl, glbData]]));

    const result = await strategyAssets("sushi", { cache: true });

    expect(result.assets["assets/sushi.glb"]!.cached).toBe(true);
    expect(cacheMockAssets.match).toHaveBeenCalledWith(assetUrl);
  });

  it("キャッシュミス時はネットワークから取得してキャッシュに保存する", async () => {
    mockFetch(sampleManifest);
    const { cacheMockAssets } = mockCaches(); // 空キャッシュ

    const result = await strategyAssets("sushi", { cache: true });

    expect(result.assets["assets/sushi.glb"]!.cached).toBe(false);
    expect(cacheMockAssets.put).toHaveBeenCalled();
  });
});

describe("strategyAssets — コールバック", () => {
  it("onProgress が各ファイル完了後に呼ばれる", async () => {
    mockFetch(sampleManifest);

    const progressEvents: number[] = [];
    await strategyAssets("combo", {
      cache: false,
      onProgress: (e) => progressEvents.push(e.completedFiles),
    });

    // combo は 2 ファイル → 少なくとも 2 回呼ばれる
    expect(progressEvents.length).toBeGreaterThanOrEqual(2);
  });

  it("onComplete が完了後に result を渡して呼ばれる", async () => {
    mockFetch(sampleManifest);

    let completeResult: unknown = null;
    await strategyAssets("sushi", {
      cache: false,
      onComplete: (r) => { completeResult = r; },
    });

    expect(completeResult).not.toBeNull();
    expect((completeResult as { name: string }).name).toBe("sushi");
  });
});

describe("strategyAssets — manifest URL オプション", () => {
  it("manifestUrl オプションが fetch に渡される", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "/custom/manifest.json") {
        return { ok: true, status: 200, json: async () => sampleManifest, body: null };
      }
      return {
        ok: true,
        status: 200,
        headers: new Headers({ "Content-Length": "4" }),
        arrayBuffer: async () => glbData,
        body: null,
      };
    });
    vi.stubGlobal("fetch", fetchMock);

    await strategyAssets("sushi", {
      cache: false,
      manifestUrl: "/custom/manifest.json",
    });

    expect(fetchMock).toHaveBeenCalledWith("/custom/manifest.json");
  });
});
