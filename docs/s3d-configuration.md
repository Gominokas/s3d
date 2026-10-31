> 出典: Notion「s3d の具体的な構成」 https://app.notion.com/p/32a6a05ca4d38130a24ef8322e2c301b
> git 化: 2026-11-01（Notion から取得した本文をそのまま収録。本リポジトリ docs/ の git 版を正とする）

---

s3d の全体像をまとめるね。これをそのまま GPT に投げて。

---

### s3d とは

s3d（Serverless Static Asset Delivery Framework）は、バックエンドを持たず、インフラ層とフロント層だけで完結する「インフロントフレームワーク」。R2/S3 を主戦場とし、静的アセットの配信戦略を宣言的に定義する。

### リポジトリ構成（Rust + TypeScript）

<b>Rust クレート:</b><br /><br />- <code>s3d-types</code> — 共通型定義（Manifest, Strategy, AssetEntry 等）<br /><br />- <code>s3d-deploy</code> — manifest の diff 計算、URL rewrite<br /><br />- <code>s3d-loader</code> — ビルド時のファイル収集、ハッシュ生成<br /><br />- <code>s3d-cli</code> — CLI 本体（init, build, push, validate, add-strategy）

<b>TypeScript パッケージ:</b><br /><br />- <code>@statics-lead/loader</code> — ブラウザ側ランタイム。strategyAssets() を提供

### CLI コマンド

<code>s3d init</code> — プロジェクト雛形生成（s3d.config.json, src/, .env.example）

<code>s3d build</code> — src/ を収集、strategy 登録ファイルにハッシュ付与、output/ に配置。manifest.json と loader.js を自動生成。ビルド時の URL は相対パス。

<code>s3d push</code> — output/ を R2/S3 にアップロード。manifest の URL を CDN 絶対パスに rewrite。diff 判定は hash AND url の両方で比較。loader.js も自動アップロード。

<code>s3d validate</code> — manifest と実ファイルの整合性チェック。

<code>s3d add-strategy</code> — 対話式で strategy.json を生成（未実装、Issue #39）。

### s3d.config.json

### strategy.json（src/assetsStrategy/cake-3d/strategy.json）

- <code>initial: false</code> → 初期表示には含めない（遅延読み込み）
- <code>initial: true</code> → 初期表示で読み込む
- <code>cache</code> → ブラウザ Cache API に保存するか
- <code>maxAge</code> → キャッシュ保持期間
- <code>reload</code> → manifest が変わったら diff で差分更新

### manifest.json（build 時生成）

- build 時は相対パス（/assets/...）
- push 時に CDN URL に rewrite（https://cdn.example.com/assets/...）

### loader.js（ブラウザ側ランタイム）

build 時に自動生成され output/ に配置。ESM 形式。

内部処理:<br /><br />1. manifest.json を fetch（Cache API でキャッシュ、buildTime でバージョニング）<br /><br />2. strategy 名から対象ファイルを解決<br /><br />3. 各ファイルを fetch（リトライ、exponential backoff、タイムアウト付き）<br /><br />4. Cache API に保存<br /><br />5. manifest が更新されたら diff で差分のみ再取得

### push の差分判定

1. R2 から旧 manifest を取得
2. ローカルの新 manifest と比較
3. hash OR url が異なるファイルをアップロード対象
4. 旧 manifest にあって新 manifest にないファイルを削除対象
5. アップロード順: assets → loader.js → manifest.json

### アセット分離の原則

- 静的アセット（サイト構造: ロゴ、3Dモデル、フォント）→ strategy.json で管理
- 動的アセット（商品写真等）→ DB 戦略で R2 直置き + CDN 配信

### provider 差し替え

s3d.config.json の provider を変えるだけで R2 ↔ S3 を切り替え。ローカルテストは build → http-server で CDN 不要。将来的に <code>push --local</code> で自前サーバー配信も対応予定。

### テスト状況

- Rust: 162 テスト pass
- TypeScript: 23 テスト pass

### 設計思想

- output/ は「発射台」。そこに載せるまでの手順をフレームワークが吸収する
- 読み取りは全て CDN で完結。サーバーレスは書き込み時のみ
- publish commit: worker が 200 を返した時点で公開状態が確定
- input / middle / output 三層モデル（DB 戦略、設計中）
- Observable strong consistency: 観測可能な公開状態に対する強整合
- Capability-based security: cookie/session ではなくプリサインド URL + Key 分離

---

これ投げて。s3d の実装を知った上でどう評価が変わるか見よう。
