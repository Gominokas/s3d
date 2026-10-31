> 出典: s3d の現行コード（mainline 46dce49）から読み取った仕様（2026-10-29、s3d サイドチャットで作成。原文は ~/workspace/s3d-notes/code-spec.md）
> git 化: 2026-11-01（本リポジトリ docs/ の git 版を正とする。元ファイルは当面残置）

---

# s3d コード仕様まとめ（コードから復元）

- 対象: Gominokas/s3d、branch genspark_ai_developer、HEAD 46dce49（Issue #41）
- 作成: 2026-10-29、ローカルクローン ~/workspace/repos/s3d を精読して復元
- 注意: Notion 仕様書ではなく、現在のコードが実際にどう動くかの仕様。設計だけ存在して未実装のものは「未実装・注意」に分けた。

## 1. 目的と全体フロー

s3d = Serverless Static Asset Delivery Framework。静的ファイル（画像・JS・CSS・3Dモデル等）を CDN へ自動デプロイし、フロントエンドから戦略的に配信するための Rust ツールチェーン。

```
src/ ─→ s3d build ─→ output/ + manifest.json ─→ s3d push ─→ Cloudflare R2 / S3互換
                                                              │
ブラウザ ─→ loader.js の strategyAssets("name") ─→ manifest を見て対象ファイルだけ CDN から取得（Cache API）
```

## 2. 利用者プロジェクトの構成

```
my-project/
├─ s3d.config.json
├─ .env（.env.example をコピーして認証情報）
├─ src/
│  ├─ index.html
│  ├─ assets/…（自由配置）
│  └─ assetsStrategy/
│     ├─ strategy.json（ルート戦略・旧形式、下記 4-2）
│     └─ <name>/strategy.json（名前付き戦略・新形式、下記 4-1）
└─ output/（build 生成物、.gitignore 済み）
   ├─ manifest.json / loader.js / ハッシュ付き・なしファイル群
```

## 3. s3d.config.json（CLI版スキーマ、crates/s3d-cli/src/config.rs の S3dCliConfig）

- 必須: project（文字列）、storage.provider（cloudflare-r2 / aws-s3 / custom、kebab-case）、storage.bucket、storage.cdn_base_url
- storage 任意: account_id（R2用）、endpoint（S3/Custom用）、region
- 任意（デフォルト）: src_dir = "src"、output_dir = "output"、include = []（glob、空=全ファイル）、exclude = []、max_file_size = なし（例 "100MB"）、manifest_path = なし（デフォルト output/manifest.json）、plugins = []（予約フィールド、JSON値の配列。中身は未使用）
- CLI は `--config`（グローバル引数、デフォルト ./s3d.config.json）でパス指定。プロジェクトルート = config ファイルのあるディレクトリ。

## 4. strategy の定義（形式が2つ共存している）

### 4-1. 名前付き戦略（新形式、#41）: src/assetsStrategy/<name>/strategy.json

フォルダ名が strategyAssets("<name>") の呼び出し名になる。build が走査して manifest.strategies に取り込む。

| フィールド | 型 | 必須 | 意味 |
|---|---|---|---|
| files | string[] | 実質必須（省略時 空配列） | CDN配信するファイル。ハッシュ付きで output にコピーされる |
| initial | string（パス） | 任意 | 初期表示ファイル。ハッシュなしでコピーされ、HTML 表示枠へ自動注入される。旧形式の bool は互換処理で無視（None扱い） |
| cache | bool | 任意（デフォルト true） | ブラウザ側 Cache API を使うか。build のパースはデフォルト true で寛容 |
| maxAge | string（例 "7d"） | 任意 | キャッシュ保持期間の宣言（下記「未実装」参照） |
| reload | {trigger: string, strategy: string} | 任意 | 例 trigger="manifest-change", strategy="diff"。両方揃わない場合は None |

パースは手書きで寛容（crates/s3d-cli/src/commands/build.rs parse_strategy_json）: files 以外は欠けてもエラーにならない。

### 4-2. ルート戦略（旧形式）: src/assetsStrategy/strategy.json

Rust版ローダー（s3d-loader の AssetsStrategyConfig）と validate / init が使う形式。build はこのファイル自体はスキップする（サブディレクトリのみ対象）。

```json
{
  "initial": { "sources": ["assets/style.css", "assets/main.js"], "cache": true, "fallback": "（任意）" },
  "cdn":     { "files": ["assets/models/**"], "cache": true, "maxAge": "7d" },
  "reload":  { "trigger": "manifest-change", "strategy": "diff", "intervalMs": 任意 }
}
```

- reload.trigger は kebab-case 列挙: manifest-change など。reload.strategy は diff / full。
- validate はこのルートファイルに initial / cdn / reload の3キーが存在することだけを必須チェックする。

## 5. manifest.json（s3d-types DeployManifest、camelCase）

```json
{
  "schemaVersion": 1,
  "version": "1.0.0",
  "buildTime": "RFC3339（build 実行時刻）",
  "assets": {
    "<元のキー>": { "url": "/assets/sushi.a1b2c3d4.glb", "size": 0, "hash": "SHA-256先頭8文字", "contentType": "model/gltf-binary", "dependencies": ["（glTF依存があれば任意）"] }
  },
  "strategies": { "<name>": { "files": [], "initial": "任意", "cache": true, "maxAge": "任意", "reload": "任意" } }
}
```

- build 時点の url はルート相対（cdn_base_url を埋め込まない）。push 時に rewrite_urls_to_cdn で CDN 絶対URLへ書き換えられてからアップロードされる。
- assets のキーは元のパス。ハッシュ付きファイル名は url の中だけに現れる。
- ハッシュ: SHA-256 hex の先頭8文字を拡張子の前に入れる（例 main.abc12345.js）。DEFAULT_HASH_LENGTH = 8。
- contentType は拡張子推定。glb=model/gltf-binary、gltf=model/gltf+json、ktx2/basis 等の3D系を優先処理し、.gltf の buffers/images 参照を dependencies に解決する。

## 6. CLI コマンド

| コマンド | 仕様 |
|---|---|
| s3d init | 対話式（dialoguer）: プロジェクト名、プロバイダー選択、バケット名、CDNベースURL、R2ならアカウントID（任意）、S3ならリージョンを聞いて s3d.config.json を生成。.env.example、.gitignore（/target・.env・output/ を追記、重複なし）、src/index.html、src/assets/.gitkeep、ルート＋sushi の strategy.json 例、output/ を生成 |
| s3d build [--no-clean] | 下記 7 の通り |
| s3d diff [--old old.json] new.json | 2つの manifest を比較し +追加 / ~変更 / -削除 / 変更なし件数を表示。old 省略時は全件追加（初回扱い）。差分判定は hash AND url の両方一致のみ Unchanged（#37） |
| s3d push [--manifest path] [--dry-run] | 下記 8 の通り |
| s3d validate | config 読込 → project/bucket/cdn_base_url の空チェック＋必須環境変数の有無 → ルート strategy.json の必須キー（initial/cdn/reload）存在 → サブディレクトリ戦略の files が src/assets/ に存在するかを警告表示。秘密情報はマスク表示 |

## 7. build の処理（crates/s3d-cli/src/commands/build.rs）

1. output/ を全クリーン（--no-clean で省略）
2. src/ を再帰収集（s3d-deploy collect）。固定除外: assetsStrategy/** と **/.gitkeep。config の include/exclude（glob、大文字小文字無視）と max_file_size を適用
3. 全ファイルの SHA-256 を計算
4. assetsStrategy のサブディレクトリを走査して strategies を構築（ルート strategy.json はスキップ、strategy.json のないフォルダも無言スキップ）
5. コピー振り分け: strategies の files に含まれるものだけハッシュ付きファイル名で output へ。それ以外（initial を含む）は元の名前のまま
6. manifest.json を書く（version は "1.0.0" 固定、strategies セクション付与）
7. loader.js を書く: バイナリ埋め込みの TS ローダーバンドル（packages/loader/loader.bundle.js を include_bytes!）。initial＋files 両方を持つ戦略には、DOMContentLoaded 後に strategyAssets(name) を呼び初期画像を差し替える自動スクリプトを末尾に追記
8. output 内 HTML の表示枠へ初期画像を自動注入: `<div id="s3d-<name>">` か `<div id="<name>">` が空の場合のみ `<img src="/<initial>" class="s3d-initial" id="s3d-img-<name>">` を入れる

## 8. push の処理（crates/s3d-cli/src/commands/push.rs + storage）

1. ローカル manifest を読み、URL を CDN 絶対URLに書き換える
2. （dry-run でなければ）loader.js を常時アップロード（manifest 外の固定ファイル）
3. リモート（ストレージ）から旧 manifest.json を取得（失敗したら初回扱い）
4. 差分計算（hash AND url）→ アップロード対象と削除対象を決定。変更なしなら早期終了（この場合 manifest 自体は再アップロードされない）
5. アップロードは並列8件ずつ。アップロードキーは manifest の URL パス部分（ハッシュ付きファイル名）。削除は Deleted のキーを storage.delete
6. 最後に書き換え済み manifest.json をアップロード
7. --dry-run 時はアップロード／削除／loader.js 転送をせず、対象一覧と件数だけ表示（リモート manifest の取得は行う）

Cloudflare の機能で使っているのは R2 のみ（コード全体を検索して確認、2026-10-29）。Workers / KV / D1 / Pages 連携のコードも wrangler.toml も存在しない（Workers＋KV は issue #29 の DB戦略の設計段階のみ）。R2 へのアクセスも Cloudflare 独自 API ではなく S3 互換エンドポイント（<account_id>.r2.cloudflarestorage.com）への SigV4 署名で、公開配信は cdn_base_url（r2.dev またはカスタムドメイン）経由。s3d-types の S3dConfig にある PagesConfig という名前も Cloudflare Pages 連携ではなく単なる出力設定の型名。

ストレージ実装: 実体は単一の R2Storage（S3互換 REST＋AWS SigV4 署名、storage/sign.rs）。provider が cloudflare-r2 の場合は endpoint デフォルト https://<account_id>.r2.cloudflarestorage.com、aws-s3 / custom の場合は endpoint 必須・region 任意で同じ実装を使う。StoragePlugin トレイト（put/get/delete/list）の実装はこれ1つのみ。

認証情報（環境変数、.env からも読む。優先順位つきフォールバック）:
- CLOUDFLARE_R2_ACCESS_KEY_ID → S3D_ACCESS_KEY_ID
- CLOUDFLARE_R2_SECRET_ACCESS_KEY → S3D_SECRET_ACCESS_KEY
- CLOUDFLARE_ACCOUNT_ID は .env.example にあるが、コードでは config の storage.account_id を使う

## 9. ブラウザローダー（TypeScript、packages/loader = @statics-lead/loader 0.1.0）

実際にブラウザで動くのはこちら。build が loader.bundle.js として output/loader.js に同梱する。

```ts
strategyAssets(name: string, options?: FetchOptions): Promise<StrategyAssetsResult>
```

- options: manifestUrl（デフォルト "/manifest.json"）、cache（デフォルト true）、onProgress({loaded,total,completedFiles,totalFiles})、onComplete(result)、timeout（デフォルト 30000ms/ファイル）、retries（デフォルト 2）
- 戻り値: { name, assets: { "<キー>": { url, data: ArrayBuffer, hash, contentType, cached } }, manifestVersion }
- 処理: manifest を取得（Cache API にバージョン付きでキャッシュし、ポインタで最新を参照）→ strategies[name] がなければ利用可能名を列挙してエラー → files を1件ずつ順に処理: Cache API ヒットならそれを返し（cached=true）、ミスなら fetch（タイムアウト＋リトライ）して Cache API に保存。assets にキーがなければ「build を再実行してください」とエラー
- strategy の cache フラグが false なら Cache API を使わない

## 10. Rust ライブラリ群（CLIからは使われないものを含む）

| クレート | 公開仕様 |
|---|---|
| s3d-types | 共通型とトレイト。DeployManifest / AssetEntry / StrategyEntry、StoragePlugin（async put/get/delete/list）、DisplayPlugin（render(&RenderContext) -> Vec<HtmlOutput>）。ほかに S3dConfig（deploy/pages/cdn/assets・display・draft を持つ別スキーマ）があるが、CLI は使っていない（s3d-display のテストと RenderContext 用） |
| s3d-deploy | パイプライン部品: collect → hash_assets → build_manifest → diff_manifests。CLI の build / diff / push が実際に使う |
| s3d-loader | Rust 版ローダー。AssetLoader::strategy_assets(manifest_url, &AssetsStrategyConfig, on_progress, token) が「旧形式」戦略（initial.sources / cdn.files）を対象に、インメモリキャッシュ（hash 一致で判定）、並列 fetch、fetch_cdn_diff（バックグラウンド差分取得）、キャンセル用 CancellationToken を提供。Service Worker 不使用・フレームワーク非依存が設計制約。CLI・ブラウザのどちらからも使われていない（ライブラリとして独立） |
| s3d-display | HTML 生成ライブラリ。<!-- s3d-part: id -->〜<!-- s3d-part-end --> マーカーで HTML をパーツ分割し iframe 用に再配置（partition_page / replace_iframe_markers）、親／パーツのテンプレート生成、出力書き出し、PlainHtmlDisplay（デフォルト DisplayPlugin 実装）。CLI からは使われていない |

依存関係: cli → types, deploy / deploy → types / loader → types / display → types, loader。

## 11. 未実装・宣言だけ・コード上の注意

1. strategy の maxAge と reload は、TS ローダーでは型定義のみで取得ロジックでは未使用。宣言してもブラウザの挙動は変わらない（キャッシュは Cache API 任せ、削除は URL のハッシュが変わることで実質的に効く）。
2. Rust ローダーの reload も、ドキュメントに「trigger に従い再フェッチ」とあるが、公開 API は strategy_assets と fetch_cdn_diff の手動呼び出しが中心で、自動リロードの常駐処理はない。
3. plugins フィールドは予約のみ。build 時の存在確認やフック実行は未実装（issue #18 の範囲）。
4. storage provider の gcs は未実装。aws-s3 も専用実装ではなく R2Storage の流用。
5. init が生成する sushi の例は "initial": false（旧 bool 形式）で、新パーサでは互換処理により initial なしとして扱われる。
6. init が生成する index.html のサンプルは `await import('./assetsStrategy/loader.js')` で引数なし strategyAssets() を呼ぶ古い書き方で、現在の TS ローダーの必須引数（name）や配置（output/loader.js）と一致していない。
7. push の個別アップロード／削除の失敗は eprintln 表示のみで、コマンド全体は失敗扱いにならない（manifest のアップロード失敗だけがエラーになる）。「変更なし」早期終了時は loader.js は転送済みだが manifest は更新されない。
8. validate のルート strategy.json チェックは旧形式前提で、新形式（サブディレクトリ）の必須項目チェックは files 存在警告のみ。
