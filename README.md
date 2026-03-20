# s3d — Serverless Static Asset Delivery Framework

**s3d** は静的ファイル（画像・JS・CSS・3D モデルなど）を CDN へ自動デプロイし、
フロントエンドから戦略的に配信するための Rust ツールチェーンです。

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
                         (assets + strategies)
                                 │
                         s3d-loader (browser)
                                 │
                         strategyAssets("name")
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

# 3. src/ にファイルを配置し、戦略を定義
#    （s3d init で src/index.html, src/assets/, src/assetsStrategy/sushi/ などが生成済み）

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
│   ├─ assetsStrategy/         ← 1アセットグループ = 1サブディレクトリ
│   │   ├─ sushi/
│   │   │   └─ strategy.json   ← strategyAssets("sushi") と一致
│   │   ├─ gari/
│   │   │   └─ strategy.json   ← strategyAssets("gari") と一致
│   │   └─ example/
│   │       └─ strategy.json
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

**SEO 不要の重いアセット（名前で呼ぶ・必要なものだけ取得・キャッシュ共有）:**

```html
<script>
  // 必要なアセットだけ名前で呼ぶ
  const sushi = await strategyAssets("sushi");
  const gari  = await strategyAssets("gari");
  // 呼ばなければ取得しない
  // 別ページで同じ名前を呼べばキャッシュヒット
</script>
```

### `src/assetsStrategy/<name>/strategy.json` の例

各サブディレクトリ名が `strategyAssets("name")` の呼び出し名と一致します。

```json
{
  "files": ["assets/sushi.glb"],
  "initial": false,
  "cache": true,
  "maxAge": "7d",
  "reload": {
    "trigger": "manifest-change",
    "strategy": "diff"
  }
}
```

フォルダ名とアセットは自由に設計できます。戦略側で「いつ取得するか」「キャッシュするか」「差分更新するか」を宣言します。

## manifest.json の構造

`s3d build` が生成する `manifest.json` には `strategies` セクションが含まれます。

```json
{
  "schemaVersion": 1,
  "version": "1.0.0",
  "buildTime": "2026-03-20T00:00:00Z",
  "assets": {
    "assets/sushi.glb": {
      "url": "https://cdn.example.com/assets/sushi.a1b2c3d4.glb",
      "size": 204800,
      "hash": "a1b2c3d4",
      "contentType": "model/gltf-binary"
    }
  },
  "strategies": {
    "sushi": {
      "files": ["assets/sushi.glb"],
      "initial": false,
      "cache": true,
      "maxAge": "7d",
      "reload": { "trigger": "manifest-change", "strategy": "diff" }
    },
    "gari": {
      "files": ["assets/gari.glb"],
      "initial": false,
      "cache": true,
      "maxAge": "7d",
      "reload": { "trigger": "manifest-change", "strategy": "diff" }
    }
  }
}
```

`s3d-loader` は `strategies[name]` を参照し、対象ファイルだけを CDN から取得します。

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
  "project": "my-service",
  "storage": {
    "provider": "cloudflare-r2",
    "bucket": "my-assets-bucket",
    "cdn_base_url": "https://cdn.example.com",
    "account_id": "your_cloudflare_account_id"
  },
  "src_dir": "src",
  "output_dir": "output",
  "plugins": []
}
```

> `plugins` フィールドは将来のプラグイン機構のために予約された空配列です。  
> 現時点では何も設定する必要はありません。

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
