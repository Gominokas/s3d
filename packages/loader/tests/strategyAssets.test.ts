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

describe("strategyAssets — 並列取得", () => {
  // 6 ファイルの strategy（並列数の検証用）
  const manyManifest: DeployManifest = {
    schemaVersion: 1,
    version: "1.0.0",
    buildTime: "2026-03-20T00:00:00Z",
    assets: Object.fromEntries(
      Array.from({ length: 6 }, (_, i) => [
        `assets/file${i}.glb`,
        {
          url: `/assets/file${i}.hash${i}.glb`,
          size: 4,
          hash: `hash${i}`,
          contentType: "model/gltf-binary",
        },
      ])
    ),
    strategies: {
      many: {
        files: Array.from({ length: 6 }, (_, i) => `assets/file${i}.glb`),
        initial: false,
        cache: true,
      },
    },
  };

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  /** 遅延つき fetch モック。同時実行数を記録する */
  function mockDelayedFetch(delayMs: number, failUrl?: string) {
    let inFlight = 0;
    let maxInFlight = 0;
    const fetchMock = vi.fn(async (url: string) => {
      if ((url as string).endsWith(".json")) {
        return { ok: true, status: 200, json: async () => manyManifest, body: null };
      }
      inFlight += 1;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await sleep(delayMs);
      inFlight -= 1;
      if (failUrl && url === failUrl) {
        return { ok: false, status: 500, statusText: "Internal Server Error", body: null };
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
    return { getMaxInFlight: () => maxInFlight, fetchMock };
  }

  it("concurrency 2 では同時実行数が 2 以下になる", async () => {
    const { getMaxInFlight } = mockDelayedFetch(20);

    const result = await strategyAssets("many", { cache: false, concurrency: 2 });

    expect(Object.keys(result.assets)).toHaveLength(6);
    expect(getMaxInFlight()).toBeLessThanOrEqual(2);
    expect(getMaxInFlight()).toBeGreaterThan(1);
  });

  it("concurrency 1 では逐次取得（同時実行数 1）になる", async () => {
    const { getMaxInFlight } = mockDelayedFetch(10);

    const result = await strategyAssets("many", { cache: false, concurrency: 1 });

    expect(Object.keys(result.assets)).toHaveLength(6);
    expect(getMaxInFlight()).toBe(1);
  });

  it("完了順がバラバラでも全ファイルがそろう", async () => {
    // ファイルごとに遅延を変えて完了順をシャッフルする
    let inFlight = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if ((url as string).endsWith(".json")) {
          return { ok: true, status: 200, json: async () => manyManifest, body: null };
        }
        const idx = Number(/file(\d)/.exec(url)?.[1] ?? 0);
        inFlight += 1;
        await sleep((5 - idx) * 10);
        inFlight -= 1;
        return {
          ok: true,
          status: 200,
          headers: new Headers({ "Content-Length": "4" }),
          arrayBuffer: async () => glbData,
          body: null,
        };
      })
    );

    const completed: number[] = [];
    const result = await strategyAssets("many", {
      cache: false,
      concurrency: 3,
      onProgress: (e) => completed.push(e.completedFiles),
    });

    expect(Object.keys(result.assets)).toHaveLength(6);
    expect(Math.max(...completed)).toBe(6);
  });

  it("1件でも失敗すれば strategy 全体が失敗する（従来と同じ意味）", async () => {
    mockDelayedFetch(5, "/assets/file2.hash2.glb");

    await expect(
      strategyAssets("many", { cache: false, concurrency: 3 })
    ).rejects.toThrow("アセット取得に失敗しました");
  });
});

describe("strategyAssets — 中断", () => {
  it("開始前に abort すればアセット fetch なしで AbortError になる", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if ((url as string).endsWith(".json")) {
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

    const controller = new AbortController();
    controller.abort();

    await expect(
      strategyAssets("combo", { cache: false, signal: controller.signal })
    ).rejects.toMatchObject({ name: "AbortError" });

    // manifest 取得は行うが、アセット fetch は呼ばれない
    const assetCalls = fetchMock.mock.calls.filter(
      ([url]) => !(url as string).endsWith(".json")
    );
    expect(assetCalls).toHaveLength(0);
  });

  it("取得中に abort すれば AbortError になり、再試行しない", async () => {
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    const assetCalls = new Map<string, number>();
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: { signal?: AbortSignal }) => {
        if ((url as string).endsWith(".json")) {
          return { ok: true, status: 200, json: async () => sampleManifest, body: null };
        }
        assetCalls.set(url, (assetCalls.get(url) ?? 0) + 1);
        // 遅延させて abort のタイミングを作る。本物の fetch と同じく、
        // シグナルが abort されたら AbortError で失敗する。
        await sleep(50);
        if (init?.signal?.aborted) {
          const err = new Error("The operation was aborted");
          err.name = "AbortError";
          throw err;
        }
        return {
          ok: true,
          status: 200,
          headers: new Headers({ "Content-Length": "4" }),
          arrayBuffer: async () => glbData,
          body: null,
        };
      })
    );

    const controller = new AbortController();
    const promise = strategyAssets("combo", {
      cache: false,
      signal: controller.signal,
    });
    await sleep(10);
    controller.abort();

    await expect(promise).rejects.toMatchObject({ name: "AbortError" });
    // 中断は再試行しないので、各 URL の fetch は 1 回だけ
    for (const count of assetCalls.values()) {
      expect(count).toBe(1);
    }
  });

  it("中断でない失敗は従来どおりリトライする", async () => {
    let attempts = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if ((url as string).endsWith(".json")) {
          return { ok: true, status: 200, json: async () => sampleManifest, body: null };
        }
        attempts += 1;
        if (attempts <= 2) {
          return { ok: false, status: 500, statusText: "Internal Server Error", body: null };
        }
        return {
          ok: true,
          status: 200,
          headers: new Headers({ "Content-Length": "4" }),
          arrayBuffer: async () => glbData,
          body: null,
        };
      })
    );

    const result = await strategyAssets("sushi", { cache: false });
    expect(result.assets["assets/sushi.glb"]).toBeDefined();
    expect(attempts).toBe(3);
  });
});

describe("strategyAssets — 完全性検証", () => {
  /** ArrayBuffer の SHA-256 先頭 8 文字（manifest の hash 形式） */
  async function sha256Fingerprint(data: ArrayBuffer): Promise<string> {
    const digest = await crypto.subtle.digest("SHA-256", data);
    const hex = [...new Uint8Array(digest)]
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    return hex.slice(0, 8);
  }

  /** hash だけ差し替えた manifest を作る */
  function manifestWithHash(hash: string): DeployManifest {
    return {
      ...sampleManifest,
      assets: {
        ...sampleManifest.assets,
        "assets/sushi.glb": {
          ...sampleManifest.assets["assets/sushi.glb"]!,
          hash,
        },
      },
    };
  }

  it("verifyIntegrity: true で正しいフィンガープリントなら通る", async () => {
    const hash = await sha256Fingerprint(glbData);
    mockFetch(manifestWithHash(hash));

    const result = await strategyAssets("sushi", {
      cache: false,
      verifyIntegrity: true,
    });

    expect(result.assets["assets/sushi.glb"]).toBeDefined();
    expect(result.assets["assets/sushi.glb"]!.data.byteLength).toBe(4);
  });

  it("verifyIntegrity: true で不一致なら失敗し、再試行もキャッシュ保存もしない", async () => {
    const { cacheMockAssets } = mockCaches(); // 空キャッシュ

    const assetCalls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if ((url as string).endsWith(".json")) {
        return { ok: true, status: 200, json: async () => manifestWithHash("00000000"), body: null };
      }
      assetCalls.push(url);
      return {
        ok: true,
        status: 200,
        headers: new Headers({ "Content-Length": "4" }),
        arrayBuffer: async () => glbData,
        body: null,
      };
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      strategyAssets("sushi", { cache: true, verifyIntegrity: true })
    ).rejects.toThrow("完全性検証に失敗しました");

    // 検証は fetchAsset のリトライの外側なので、アセット fetch は 1 回だけ
    expect(assetCalls).toHaveLength(1);
    // 破損したデータをキャッシュに入れない
    expect(cacheMockAssets.put).not.toHaveBeenCalled();
  });

  it("verifyIntegrity が off（既定）なら不一致でも通る", async () => {
    mockFetch(manifestWithHash("00000000"));

    const result = await strategyAssets("sushi", { cache: false });

    expect(result.assets["assets/sushi.glb"]).toBeDefined();
  });

  it("キャッシュヒットが不一致なら evict して再取得する", async () => {
    const hash = await sha256Fingerprint(glbData);
    const manifest = manifestWithHash(hash);
    // fetch は正しいデータ（glbData）を返す
    mockFetch(manifest);

    // キャッシュには別の中身（badData）を入れておく
    const badData = new Uint8Array([1, 2, 3, 4]).buffer;
    const assetUrl = "/assets/sushi.abcd1234.glb";
    const { cacheMockAssets, assetStore } = mockCaches(
      new Map([[assetUrl, badData]])
    );

    const result = await strategyAssets("sushi", {
      cache: true,
      verifyIntegrity: true,
    });

    // 再取得した正しいデータが返る（キャッシュヒットではない）
    expect(result.assets["assets/sushi.glb"]!.cached).toBe(false);
    expect(
      new Uint8Array(result.assets["assets/sushi.glb"]!.data)
    ).toEqual(new Uint8Array(glbData));
    // 古いキャッシュは evict され、新しいデータで保存し直される
    expect(cacheMockAssets.delete).toHaveBeenCalledWith(assetUrl);
    expect(cacheMockAssets.put).toHaveBeenCalled();
    expect(assetStore.get(assetUrl)).toEqual(glbData);
  });

  it("キャッシュヒットが一致すれば検証してそのまま返す", async () => {
    const hash = await sha256Fingerprint(glbData);
    const manifest = manifestWithHash(hash);

    const fetchMock = vi.fn(async (url: string) => {
      if ((url as string).endsWith(".json")) {
        return { ok: true, status: 200, json: async () => manifest, body: null };
      }
      throw new Error("アセット fetch が呼ばれてはならない");
    });
    vi.stubGlobal("fetch", fetchMock);

    const assetUrl = "/assets/sushi.abcd1234.glb";
    mockCaches(new Map([[assetUrl, glbData]]));

    const result = await strategyAssets("sushi", {
      cache: true,
      verifyIntegrity: true,
    });

    expect(result.assets["assets/sushi.glb"]!.cached).toBe(true);
    expect(result.assets["assets/sushi.glb"]!.data.byteLength).toBe(4);
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
