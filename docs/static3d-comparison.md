> 出典: static3d と s3d の実装比較調査（2026-10-31。原文は ~/workspace/s3d-notes/static3d-comparison.md）
> git 化: 2026-11-01（本リポジトリ docs/ の git 版を正とする。元ファイルは当面残置）

---

# static3d（旧・TS版）と s3d（現行・Rust版）の実装比較

- 比較日: 2026-10-31
- static3d: Gominokas/static3d、main の HEAD 4f50e94、ローカルクローン ~/workspace/repos/static3d（読み取り専用で取得）
- s3d: HEAD 46dce49（+ LICENSE の PR #43）
- 結論: コアのパイプラインは移植済みだが、static3d に実装済みで s3d に無いものが複数ある。「全部できてる」は誤り。

## s3d に移植済み（同等あり）

| 機能 | static3d | s3d |
|---|---|---|
| 収集・ハッシュ・manifest 生成 | packages/deploy/src/build（collect/hash/manifest、SHA-256 先頭8文字） | s3d-deploy + CLI build（同方式） |
| 差分・push（R2、差分アップ＋削除）・validate | deploy の push/diff、cli/validate | CLI push/diff/validate（差分は hash AND url） |
| ブラウザローダー | @static3d/loader の AssetLoader（Blob の Map を返す） | TS版 strategyAssets（ArrayBuffer の Record を返す）＋ Rust版 s3d-loader |
| glTF 依存の抽出 | build/gltf.ts | s3d-deploy manifest の dependencies 解決 |
| 型（config/manifest） | @static3d/types | s3d-types に移植。ただし CLI は使っていない型が多い（下記） |

## static3d にあって s3d に無いもの

| # | 機能 | static3d の実装 | s3d の状態 |
|---|---|---|---|
| 1 | GLB 最適化 | build/optimize.ts: glTF-Transform で dedup / prune / Draco 圧縮、config の optimize で個別 on/off、サイズ削減ログ | 無し。s3d はハッシュしてコピーするだけ |
| 2 | Vite プラグイン＋dev server | vite/plugin.ts・devServer.ts・devManifest.ts: dev 時は deferred/ から manifest 動的生成、/cdn/* 配信、fs.watch で無効化。build 時は closeBundle でパイプライン実行 | 無し（候補 issue #7 の serve/watch と一部重なるが、Vite 統合そのものは別物） |
| 3 | 環境の概念と promote / admit | config の environments（draft/staging/production = R2 プレフィックス）。promote は環境間コピー（R2内 CopyObject 含む）、admit は環境からローカルへ差分ダウンロード | 無し。s3d に環境の概念自体が無い |
| 4 | Cloudflare Pages デプロイ | push/pages.ts: Cloudflare API を直接叩く wrangler 互換アップロード（バケット分割・リトライ） | 無し。s3d の Cloudflare 利用は R2 のみ（2026-10-29 確認済み） |
| 5 | 旧世代クリーンアップ | push/cleanup.ts: cdnBaseUrl のパスプレフィックス内に限定して旧世代アセットを削除。config の oldVersionRetention / oldVersionMaxAge が駆動 | 無し。s3d push は現行 manifest の diff で Deleted になったものだけ削除。型だけ s3d-types に残っている |
| 6 | Display 層（React + three.js） | @static3d/display: useAsset / useAssetProgress / AssetProvider、Stage / Room / Spot / Overlay / ProductCard / ProductOverlay / CameraEngine / useCameraTransition、types の scene.ts | 無し。s3d-display は HTML 分割と iframe 再配置のライブラリで、React も three.js シーン部品も無い（名前が同じだけで別物） |
| 7 | ローダーの並列取得と設定駆動 | display.loader 設定（concurrency / retryCount / retryBaseDelay / timeout）をローダーが参照 | TS版は逐次取得・固定デフォルト（timeout 30s / retries 2）。候補 issue #6 と一致 |

## 備考

- static3d の config スキーマ（deploy.pages/cdn/assets、display、draft、optimize、environments）は、s3d-types の S3dConfig の原型。s3d の CLI が実際に読むのは別の簡略スキーマ（S3dCliConfig）で、pages / draft / oldVersionRetention 等は型止まり。
- static3d 側の branches: main の他に genspark_ai_developer / genspark_ai_developer_v2 / local-test がある。比較は main で実施。
