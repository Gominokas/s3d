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
[output/]  ─→  s3d-deploy  ─→  manifest.json
                   │
                   ▼
              s3d-cli push  ─→  Cloudflare R2 / AWS S3
                   │
                   ▼
           s3d-loader (browser)  ─→  assetsStrategy / strategyAssets
                   │
                   ▼
           s3d-display  ─→  HTML + iframe 正規化
```

## クイックスタート

```bash
# 1. プロジェクト初期化（インタラクティブ）
s3d init

# 2. .env に認証情報を記入
cp .env.example .env
# S3D_ACCESS_KEY_ID / S3D_SECRET_ACCESS_KEY を設定

# 3. アセットを output/ に配置
cp -r dist/ output/

# 4. ビルド（マニフェスト生成）
s3d build

# 5. 差分確認（オプション）
s3d diff --old old-manifest.json output/manifest.json

# 6. R2/S3 へアップロード
s3d push

# 7. ドライラン（実際にはアップロードしない）
s3d push --dry-run
```

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
  "output_dir": "output",
  "include": [],
  "exclude": ["**/.DS_Store"]
}
```

## 環境変数

| 変数名 | 説明 |
|---|---|
| `S3D_ACCESS_KEY_ID` | R2/S3 アクセスキー ID |
| `S3D_SECRET_ACCESS_KEY` | R2/S3 シークレットアクセスキー |

## ビルド

```bash
# 全クレートのビルド
cargo build

# テスト
cargo test

# CLI バイナリのビルド
cargo build -p s3d-cli --release
```

## ライセンス

MIT
