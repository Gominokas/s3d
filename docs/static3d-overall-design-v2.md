> 出典: Notion「static3d 全体設計書 v2（確定版）」 https://app.notion.com/p/30c6a05ca4d381e4ab38fb2de4b746d9
> git 化: 2026-11-01（Notion から取得した本文をそのまま収録。本リポジトリ docs/ の git 版を正とする）

---

了解♡ じゃあ一気にいく。まず全体設計書を確定させて、そのままPhase 1のdeploy実装に入る。

---

## 1. プロダクト概要

静的な3DWebサイトを簡単に開発・デプロイできるフレームワーク群。<b>Deploy・Display・Draft</b>の3パッケージで構成され、それぞれ独立して使えるが、組み合わせると最大の効果を発揮する。本番環境にサーバーサイドコンポーネントは一切なし。

---

## 2. パッケージ構成

### @static3d/types（共有型定義）

マニフェストの型定義とJSONスキーマのみを提供する超軽量パッケージ。実装コードゼロ。deploy・display・draftの全てがここを参照する。

### @static3d/deploy（配信基盤 — Node専用）

静的サイトのアセットを最適に配信するためのCLIツールとViteプラグイン。Three.jsに依存しない。どんな静的サイトでも使える汎用ツール。

責務はアセット収集・コンテンツハッシュ付与・マニフェスト生成、.gltf内のuri再帰書き換え、CDN（R2）への差分アップロード、Cloudflare Pagesへのデプロイ、旧世代アセットのクリーンアップ、CORS設定の自動適用、デプロイの原子性保証、Viteプラグイン（ビルド時のアセットパイプライン統合・開発時のローカルCDN代替配信）。

### @static3d/display（表示レイヤー — ブラウザ専用）

3D空間を使ったWebサイトの表示体験を提供するReact/R3Fフレームワーク。

責務はランタイムローダー（fetch・進捗・リトライ・整合性検証・キャンセル）、Reactフック（useAsset・useAssetProgress）、3D空間ルーティング（Stage/Room/Spot/Overlay）、カメラ遷移エンジン、インタラクションシステム（ハイライト・クリック・ホバー）。

開発時のマニフェストは簡易版を自前生成する（ハッシュなし・ローカルURL）。本番マニフェストはdeployが生成したものを読む。マニフェストの互換性はschemaVersionで保証する。

### @static3d/draft（レビュー環境 — 将来課金候補）

deploy + displayの上に乗り、クライアントとのアセット確認・要件合意に特化した環境を提供する。Phase 3で開発。

---

## 3. 依存関係

deployとdisplayは完全に独立。typesだけを共通の契約として共有する。draftのみが両方に依存する。

---

## 4. 設定ファイル

1ファイルに全パッケージの設定を集約する。使わないセクションは省略可。

ファイル名は <code>static3d.config.json</code>。各パッケージは自分のセクションだけ読み、他のセクションには触らない。

---

## 5. 開発ロードマップ

### Phase 1: @static3d/types + @static3d/deploy

配信基盤を固める。これが全ての土台。

types: DeployManifest型定義、schemaVersion。<br /><br />deploy: CLIツール（<code>static3d deploy build</code> / <code>static3d deploy push</code> / <code>static3d deploy validate</code>）、アセット収集・ハッシュ・マニフェスト生成、.gltf uri再帰書き換え、R2差分アップロード・疎通確認・Pagesデプロイ、旧世代クリーンアップ、Viteプラグイン、設定バリデーション・認証情報解決。

完了基準: <code>vite build</code> でdist/pages + dist/cdnが生成され、<code>static3d deploy push</code> でCloudflareに配置される。

### Phase 2: @static3d/display

表示レイヤーを構築。

ランタイムローダー（AssetLoader）、Reactフック、Stage/Room/Spot/Overlay、カメラ遷移エンジン、deploy連携（deployのマニフェストを読んでCDN URLを解決）。

完了基準: <code>vite dev</code> でローカル開発でき、<code>vite build && static3d deploy push</code> で本番に出せ、URLでカメラが遷移し、Spotクリックでインタラクションが動く。

### Phase 3: @static3d/draft

レビュー環境を載せる。

プロジェクト雛形生成、アセットレビュー用ビューアー、仮デプロイ（共有URL生成）、Goサイン済みアセットのエクスポート。

---

## 6. パッケージ間連携

連携点はマニフェストのJSONファイル1つだけ。

deployが <code>dist/pages/manifest.json</code> を生成する。displayはそのmanifest.jsonを読んでCDN URLを解決しfetchする。開発時はdisplayが簡易マニフェスト（ハッシュなし・ローカルURL）を自前生成し、本番ビルド時はdeployが正式マニフェストを生成する。両者の互換性はschemaVersionで保証する。

---

## 7. ディレクトリ構成（モノレポ）

---

## 8. npmパッケージ

インポートパス：

---

## 9. 技術スタック

types: TypeScript。<br /><br />deploy: TypeScript、Node.js、Vite Plugin API、Cloudflare API（R2 + Pages）、crypto（SHA-256）。<br /><br />display: TypeScript、React、React Three Fiber、Three.js、React Router v6、@react-three/drei。<br /><br />draft: 未定（Phase 3）。<br /><br />モノレポ管理: pnpm workspaces + Turborepo。

---

これが確定版♡ じゃあPhase 1に入る。

---

# Phase 1 実装: @static3d/types + @static3d/deploy

## まずモノレポを立ち上げる

<code>package.json</code>:

<code>pnpm-workspace.yaml</code>:

<code>turbo.json</code>:

---

## @static3d/types

<code>packages/types/package.json</code>:

<code>packages/types/tsconfig.json</code>:

<code>packages/types/src/manifest.ts</code>:

<code>packages/types/src/config.ts</code>:

<code>packages/types/src/index.ts</code>:

---

## @static3d/deploy

<code>packages/deploy/package.json</code>:

<code>packages/deploy/tsconfig.json</code>:

<code>packages/deploy/src/config/schema.ts</code>:

<code>packages/deploy/src/build/collect.ts</code>:

<code>packages/deploy/src/build/hash.ts</code>:

<code>packages/deploy/src/build/gltf.ts</code>:

<code>packages/deploy/src/build/manifest.ts</code>:

<code>packages/deploy/src/build/output.ts</code>:

<code>packages/deploy/src/build/index.ts</code>:

<code>packages/deploy/src/cli/build.ts</code>:

<code>packages/deploy/src/cli/index.ts</code>:

<code>packages/deploy/src/vite/plugin.ts</code>:

---

ここまでがPhase 1のコード全量♡ この時点で動くのは：

1. <code>pnpm install</code> → モノレポセットアップ
2. <code>pnpm -r build</code> → types → deploy の順でビルド
3. テスト用プロジェクトで <code>static3d-deploy build</code> → dist/pages + dist/cdn 生成
4. Viteプラグインで <code>vite dev</code> → ローカルCDN代替配信 + 簡易マニフェスト

push（R2/Pagesデプロイ）はまだTODOだけど、ビルドパイプラインは完動する状態。これでまずファイルを作って動かしてみる？
