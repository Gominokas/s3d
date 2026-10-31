# s3d 仕様ドキュメント

s3d / static3d の仕様書は本ディレクトリ（git）で管理し、**git 版を正**とする（2026-11-01 ダーリン決定）。各文書の冒頭に出典（Notion URL 等）を明記している。元の Notion ページやノートは、ダーリンの判断があるまで削除しない。

## 中核ドキュメント

| 文書 | 内容 |
|---|---|
| [static3d-overall-design-v2.md](static3d-overall-design-v2.md) | static3d 全体設計書 v2（確定版）。Deploy / Display / Draft の全体設計 |
| [statics-lead-design-2026-03-13.md](statics-lead-design-2026-03-13.md) | Statics Lead 設計ドキュメント（2026-03-13 最終版）。DB 戦略・Draft ビジュアルエディタ等 |
| [s3d-configuration.md](s3d-configuration.md) | s3d の具体的な構成 |

## s3d の概念と現行仕様

| 文書 | 内容 |
|---|---|
| [concept.md](concept.md) | s3d の概念定義（simple-static-site-deployer、Deploy / Display / Draft の3つの強み） |
| [code-spec.md](code-spec.md) | 現行コードから読み取った仕様（CLI・strategy・manifest・ローダー） |
| [static3d-comparison.md](static3d-comparison.md) | static3d と s3d の実装比較（移植済み / 未移植の棚卸し） |

## 参照

| 文書 | 内容 |
|---|---|
| [spec-references.md](spec-references.md) | Notion 仕様ページ21件の一覧。中核以外のページはここから段階的に git 化する |
