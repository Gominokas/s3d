# s3d — Static 3D Asset Deployment

**s3d** は 3D アセット（`.glb`, `.gltf`, `.ktx2` 等）の CDN デプロイを自動化する Rust ツールチェーンです。

## クレート構成

| クレート | 説明 |
|---|---|
| [`s3d-types`](crates/s3d-types/) | 共通型定義（`CollectedAsset`, `HashedAsset`, `DeployManifest`, `StoragePlugin` トレイトなど） |
| [`s3d-deploy`](crates/s3d-deploy/) | アセット収集・SHA-256 ハッシュ化・マニフェスト生成・差分計算 |
| [`s3d-loader`](crates/s3d-loader/) | フロントエンド向けアセットローダー（assetsStrategy / strategyAssets） |
| [`s3d-display`](crates/s3d-display/) | HTML 生成・iframe 正規化（DisplayPlugin トレイト） |
| [`s3d-cli`](crates/s3d-cli/) | CLI バイナリ `s3d`（init / build / diff / push / validate） |

## アーキテクチャ

```
[src/]  ─→  s3d build  ─→  [output/]  ─→  s3d push  ─→  Cloudflare R2 / AWS S3
                                 │
                                 ▼
                         manifest.json
                                 │
                         s3d-loader (browser)
                                 │
                         assetsStrategy / strategyAssets
```

## 開発者体験 — プロジェクト作成から配信まで

```bash
mkdir my-service
cd my-service

# 1. プロジェクト初期化（インタラクティブ）
s3d init

# 2. .env に認証情報を記入
cp .env.example .env
# CLOUDFLARE_R2_ACCESS_KEY_ID / CLOUDFLARE_R2_SECRET_ACCESS_KEY を設定

# 3. src/ にファイルを配置
#    （s3d init で src/index.html, src/assets/, src/assetsStrategy/strategy.json が生成済み）

# 4. ビルド（マニフェスト生成 + ハッシュ付きファイルを output/ にコピー）
s3d build

# 5. R2/S3 へアップロード
s3d push

# → R2 で配信完了 🎉
```

### `s3d init` 後のディレクトリ構成

```
my-service/
├─ s3d.config.json
├─ .env.example
├─ .gitignore                  (/target, .env, output/ を含む)
├─ src/
│   ├─ index.html              ← スキャフォールドテンプレート
│   ├─ assetsStrategy/
│   │   └─ strategy.json       ← 配信戦略の定義
│   └─ assets/                 ← 自由に配置
│       ├─ style.css
│       ├─ main.js
│       ├─ hero.png
│       └─ models/
│           └─ shop.glb
└─ output/                     ← s3d build の出力（自動生成、.gitignore 済み）
    ├─ manifest.json
    └─ ...
```

### アセット配信の 2 つの方法

**SEO 対象ファイル（クローラに読ませる）:**

```html
<img src="assets/hero.png" alt="hero" />
<link rel="stylesheet" href="assets/style.css" />
```

**SEO 不要の重いアセット（クローラはスキップ、CDN から非同期取得）:**

```html
<script type="module">
  const { strategyAssets } = await import('./assetsStrategy/loader.js');
  const assets = await strategyAssets();
  // assets.get('assets/models/shop.glb') → CDN URL
</script>
```

### `src/assetsStrategy/strategy.json` の例

```json
{
  "initial": {
    "sources": ["assets/style.css", "assets/main.js", "assets/hero.png"],
    "cache": true
  },
  "cdn": {
    "files": ["assets/models/**", "assets/detail-*.png"],
    "cache": true,
    "maxAge": "7d"
  },
  "reload": {
    "trigger": "manifest-change",
    "strategy": "diff"
  }
}
```

ファイル配置は自由。戦略側で「最初に送るもの」「後から送るもの」を宣言します。

## CLI コマンド

| コマンド | 説明 |
|---|---|
| `s3d init` | インタラクティブにプロジェクトを初期化 |
| `s3d build` | `src/` を収集・ハッシュ化し `output/` にビルド |
| `s3d diff --old old.json new.json` | 2 つのマニフェストを比較・表示 |
| `s3d push [--dry-run]` | `output/` を R2/S3 へアップロード |
| `s3d validate` | `s3d.config.json` / `strategy.json` / 環境変数を検証 |

## s3d.config.json

```json
{
  "project": "my-3d-project",
  "storage": {
    "provider": "cloudflare-r2",
    "bucket": "my-assets-bucket",
    "cdn_base_url": "https://cdn.example.com",
    "account_id": "your_cloudflare_account_id"
  },
  "src_dir": "src",
  "output_dir": "output"
}
```

## 環境変数

| 変数名 | 説明 |
|---|---|
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare アカウント ID |
| `CLOUDFLARE_R2_ACCESS_KEY_ID` | R2 アクセスキー ID |
| `CLOUDFLARE_R2_SECRET_ACCESS_KEY` | R2 シークレットアクセスキー |
| `S3D_ACCESS_KEY_ID` | 汎用アクセスキー（フォールバック） |
| `S3D_SECRET_ACCESS_KEY` | 汎用シークレットキー（フォールバック） |

## ビルド

```bash
# 全クレートのビルド
cargo build

# テスト
cargo test

# CLI バイナリのビルド (release)
cargo build -p s3d-cli --release
```

## ライセンス

MIT
