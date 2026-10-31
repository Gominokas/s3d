> 出典: Notion「Statics Lead 設計ドキュメント（2026-03-13 最終版）」 https://app.notion.com/p/3226a05ca4d381729a03d7b41bcb9e93
> git 化: 2026-11-01（Notion から取得した本文をそのまま収録。本リポジトリ docs/ の git 版を正とする）

---

楽しかった。ごみちゃんと一緒に設計を殴り合いで磨き上げるの、最高に気持ちいい。

じゃあまとめる。

---

### 憲法

<b>1. Canonical read distribution is static artifacts.</b><br /><br />正系の読み取りデータ配信は、静的artifactで行う。

<b>2. Dynamic plugins may assist correctness or latency, but never replace the canonical read model.</b><br /><br />動的プラグインは正確性や速度を補助してよいが、正系のread modelを置き換えてはならない。

<b>3. Any requirement that turns dynamic paths into the primary query path is outside Statics Lead's intended operating envelope.</b><br /><br />動的パスを主要なクエリ経路にする要件は、Statics Leadの設計範囲外である。

<b>4. Dynamic plugins are tolerated as transitional escape hatches, and should inform future expansion of the static core.</b><br /><br />動的プラグインは過渡的な逃がし口として許容され、その利用実績は将来の静的コア拡張に反映されるべきである。

---

### 設計原則（9条）

<b>原則1. Outputはartifactであり、DBではない。</b><br /><br />read-only、immutable、versioned、再生成可能、使い捨て可能。壊れたらInputから再生成する。

<b>原則2. フロントはartifact loaderであり、query clientではない。</b><br /><br />サーバに問い合わせない。クエリを組み立てない。取得・検証・キャッシュ・表示に専念する。

<b>原則3. Inputがtruth、middle stateはreplayable derivative。</b><br /><br />InputDBが一次真実。middleDB（projection state store）はInputから再構築可能な中間状態。壊れたらreplayで復旧する。

<b>原則4. Projectionにはstatelessとstatefulの2種類がある。</b><br /><br />stateless projection（JOIN展開、lookup解決）とstateful projection（在庫残高、売掛残高等の累積計算）を明確に分離する。

<b>原則5. Stateful projectionはincremental maintenanceを許可する。</b><br /><br />毎回full rebuildしない。イベント適用で更新し、snapshotを保持してよい。ただしreplay可能であること。

<b>原則6. 静的がLead、動的がFollow。</b><br /><br />動的プラグインはコマンド処理と配信制御に介入できるが、read modelの正系になってはならない。動的Inputは許容しない。

<b>原則7. エッジコンピューティングの思想を崩さない。</b><br /><br />データをユーザーの近く（CDN/IndexedDB）に置き、中央への問い合わせを最小化する。強整合のために中央問い合わせを増やす方向の拡張は、確定画面のFollowプラグインを除き、行わない。

<b>原則8. スナップショットベースの表示が業務の実態に合致している。</b><br /><br />多くの業務画面において、常時リアルタイム変動表示は不要であり、むしろ有害な場合がある。スナップショット表示を正とし、データ鮮度（generation/timestamp）を画面に明示する。リアルタイム性はUX補助としてのみ許容する。

<b>原則9. 守備範囲を超えるものは対応しない。</b><br /><br />中途半端に対応するより、できないと明示する方がフレームワークとして信頼される。

---

### 守備範囲（5段階）

<b>A. ネイティブ（静的、Statics Lead単体）</b><br /><br />パブリックデータ配信、管理画面閲覧、ECカタログ、定型ダッシュボード（aggregate projection）、中低頻度更新の業務画面、tenant/role単位のアクセス制御（Output事前分割）、スナップショットベースの業務データ表示。<br /><br />採用基準：canonical readが静的artifactのみで成立する。

<b>B. Followプラグイン — 軽度（特定操作時のみ介入）</b><br /><br />確定画面でのInputDB直接読み取り（強整合）、書き込み確定時のバリデーション（排他チェック、重複チェック）、書き込み時の楽観的ロック、配信時の単純row/column認証フィルタ。<br /><br />採用基準：動的チェックが短命であり、汎用クエリ経路にならない。

<b>C. Followプラグイン — 重度（常時接続を伴う）</b><br /><br />可視化高速化のためのブラウザ間P2P通信。強整合のためではなく、correctness/durability/authorizationの前提にならない。なくてもアプリは正常動作する。<br /><br />採用基準：動的accelerationが存在するが、correctnessは正系パイプラインで解決される。

<b>D. 外部連携（External Companion Systems）</b><br /><br />P2Pエンジンによる投稿物の共同作成（Input投入前の作成フェーズ）、外部ワークフローシステム、外部署名サービス等。<br /><br />採用基準：機能を意図的に別のシステムに委譲している。

<b>E. スコープ外</b><br /><br />分散トランザクション、複雑ABAC、即席クエリ・BI、常時同期型リアルタイムエディタ、リアルタイム変動表示が業務要件であるシステム。<br /><br />採用基準：動的パスが主要経路になる、またはアーキテクチャの主従が逆転する。

---

### アーキテクチャ3層

<b>Input layer</b> — facts/commandsの受け口。正規化されたRDB（RDS、DynamoDB、PlanetScale等、何でも選択可能）。Worker経由で書き込み。

<b>Projection state layer（middleDB）</b> — stateful projectionの中間状態を保持。在庫残高、売掛残高、ポイント残高等の累積計算。Inputから再構築可能。業務DBではなく、projection state store。必要な場合のみ導入。

<b>Output artifact layer</b> — immutableなchunked read artifacts。R2に配置されCDN経由で配信。ブラウザのIndexedDBにキャッシュされ、ローカルで検索・フィルタ・ソートを実行。

構成パターンは2つ。Level 1: Direct Projection（Input→Output、stateless、DB不要）。Level 2: Stateful Projection（Input→Projection State→Output、累積計算あり、state store使用）。

---

### フロント主導DB設計（スキーマ駆動）

独自スキーマ言語でInputDB、middleDB、OutputDBを定義。スキーマからWorker、IndexedDBスキーマ、バリデーションを自動生成。DSLの責務はInput構造、Output構造、マスタ参照解決、ライフサイクル、チャンク分割の5つに限定。

Outputは3種に分離。entity projection（一覧・詳細画面用の行データ）、lookup projection（マスタ参照用の小さい辞書）、aggregate projection（件数・合計・日次集計）。

スキーマにlookup: snapshot \| latestを明示し、マスタ参照の時点固定を宣言可能。projection dependency graphをスキーマの参照関係から自動導出し、マスタ変更時の再生成対象を自動算出。

ファイルは機能単位で分割し、schema/sales/input.schema、schema/sales/output.schema等のディレクトリ構成。スキーマ定義がそのまま仕様書になる。

---

### データライフサイクル

パブリックデータ：p（即時差分、投稿ごとに独立JSONファイル）→ P（定時マージ済みホットデータ）→ lP（ウォーム、低頻度参照）→ 削除（過去データはOutput履歴DBの世代管理で保持）。

認証付きデータ：a → A → lA（同様の階層構造、Worker経由で認証後に取得）。

降格タイミングはconfigで設定。ビルド時にWorkerのcronトリガーを自動生成。

---

### p（即時差分）の設計

投稿ごとに独立したJSONファイルとしてR2に配置（例：p/sales/1710312345.json）。1投稿1ファイルで書き込み競合を回避。フロントはポーリングで新着pを逐次非同期fetchしIndexedDBにマージ。

Pマージ完了後、R2側はWorkerがpファイルを削除、フロント側はP取得完了後にローカルのpを破棄してPで上書き。P書き込み成功後にpを消す順序を厳守。異常終了に備え次回起動時にクリーンアップ処理を実行。

---

### Pのチャンク分割と世代管理

Pの丸替えは非許容。チャンク分割し（例：chunk: \{ size: 500, key: "sale\_date" \}）、更新があったチャンクだけを再生成。世代番号付きimmutableファイルとして配置（例：P/sales/gen-000123/page-001.json）。旧世代は削除せず履歴として保持。

manifestに各チャンクのgeneration、hash、includes\_through\_seqを持たせ、フロントは差分チャンクだけを取得。manifestを論理コミット単位とし、クライアントはmanifest更新時点でのみ採用。

Output履歴DBの保持期間はconfigで設定（例：retention: \{ generations: 100, max\_age: "5year" \}）。期限切れの世代はLifecycleWorkerが自動掃除。保持期間を超えた過去データはStatics Leadのスコープ外であり、必要なら別途アーカイブアプリケーションを構築する。

---

### 整合性メタデータ

manifestにgeneration（P世代番号）、includes\_through\_seq（Pが取り込んだpの最終シーケンス）、projection\_version（スキーマ変更時のバージョン）、checksum（チャンクごとのハッシュ）を持たせる。pには単調増加シーケンス、tombstone対応。

全データファイルを世代付きimmutableファイル名で配置し、manifestのみshort TTL / no-cache。immutable data + mutable manifestの構成を厳守。

ブラウザ保存の健全性監視として、起動時にstorage estimate、generation mismatch、partial corruption detectionを実行し、異常時はself-heal。

---

### tier連動キャッシュ・優先度制御

スキーマのtier定義（p / P / lP、a / A / lA）から、CDN側とブラウザ側の両方の挙動を自動制御。CDN側ではWorkerがR2配置時にtierに応じたCache-Controlヘッダーを付与。ブラウザ側ではfetch APIのpriority属性をtierに連動。ポーリング間隔もtierで変動。開発者はスキーマにtierを一行書くだけ。

---

### Worker分割

すべてスキーマから自動生成。InputWorker（受信・バリデーション・InputDB書き込み・Output生成・R2配置）、LifecycleWorker（p→P→lPの降格処理、cron駆動）、AuthWorker（認証・署名付きURL発行）。役割ごとに分離。

---

### 認証付きデータの取り扱い

パブリックアセットはR2パブリックバケットから認証なしで直接取得。セキュアデータはWorker経由または署名付きURLで取得。ブラウザキャッシュ（IndexedDB）への保存可否はconfigで設定可能にし、判断は顧客のセキュリティポリシーに委ねる。端末セキュリティはデバイス管理の責務。

認証データのACLは3段階。tenant/role単位事前分割（ネイティブ、Output設計で対応）、単純row/column filter（Followプラグイン、Worker認証フィルタ）、複雑ABAC（別サービス連携、今後設計）。

---

### 強整合の定義

アプリ全体に強整合を要求しない。通常閲覧は静的Output（結果整合、CDN配信）。確定画面（在庫引当、座席予約、決済確定等）だけFollowプラグインがcanonical InputDBを直接読み書きし、当該操作について強整合な確認・確定を提供する。確定後のデータは静的パイプラインに還元。

---

### useAsset段階ロード

manifestにpreview（軽量版）とfull（フル品質版）を定義。useAsset('models/shop.glb')で軽量プレビューを即時表示し、裏でフル品質版をCDNからダウンロードし完了後に自動差し替え。ビルド時にフルアセットからプレビューを自動生成。3Dだけでなく画像・JSON・SQLite等あらゆるアセット種別に対応。コアのバランシングロジックはRust/WASMに実装し、フレームワークアダプタはレンダリングのみ担当。

---

### フレームワーク非依存化

コアロジックをRust/WASMに移行し、フレームワーク別の薄いアダプタを提供。@statics-lead/display-react、@statics-lead/display-astro、@statics-lead/display-svelte、@statics-lead/display-vue、@statics-lead/display-vanilla。useAssetの同期非同期バランシング体験はどのアダプタでも同一。

---

### immediate/deferred統一

useAsset1本でバンドル内即時取得/CDN遅延取得を自動判別するように統一。Viteプラグインでimmediateアセットもmanifestに登録し、ローダーがローカルパス→CDN URLのフォールバックで解決。

---

### SPA構成とメンテナンス

History APIでURL書き換え、初回ロード後はサーバアクセスなし。差分検知はmanifestハッシュ比較。メジャーアップデートはバッチでIndexedDB再構築し、更新中は旧データで継続利用可能。メンテナンスモードはmanifestにフラグ追加、フロントで書き込みUIのみ非表示。バックエンド側メンテナンス中も読み取りは継続。

---

### プラグインシステム

Followプラグインの運用規律として、静的コアで未吸収の要件を一時的に扱うために使う。利用頻度の高いプラグイン要件は静的標準機能への昇格候補として記録・分析する。プラグインなしでもアプリは動く。プラグインは拡張であって前提条件ではない。

---

### インフラ非依存性

InputDBの選択は完全に自由。OutputとフロントにInputDB側の変更は影響しない。teshのland/shrub/envdiffでインフラ構成を切り替え、同一スキーマから異なるインフラ向けのWorkerコードを生成可能。特定クラウドベンダーへのロックインが発生しない。

---

### Draftビジュアルエディタ（セリナ提案）

TauriアプリのDraft上で、InputDBとOutputDBをノードベースGUIで繋げるビジュアルエディタ。フィールド同士を線で繋ぐとスキーマファイルが自動生成される。ライフサイクル設定もスライダーで操作可能。Input→Output生成のリアルタイムプレビューにより、テストデータを入れてOutputの形をその場で確認できる。

---

### Statics Leadとは何か

Statics Leadは、InputDBから決定論的に生成されたread-only projection artifact群をCDN配信し、ブラウザ内IndexedDBをread planeとして使う、守備範囲限定型の静的アプリケーションフレームワークである。

静的で十分なものを、もう一度静的に取り戻す。動的は必要な場所だけに限定し、その必要性すら将来は静的へ回収していく。

サイト作成者、ブラウザ作成者、ユーザーが、それぞれの目、それぞれの手で、Web空間をより良いものにするための設計思想である。

---

### Firebaseとの差異

Firebaseは動的アプリ向けBaaSで、Firestoreへの毎回クエリが前提。Statics Leadは読み取りにDBを使わず、OutputをCDN配信する。スキーマ駆動のInput→Output自動生成、データライフサイクル、useAsset段階ロード、ベンダー非依存性はFirebaseでは実現できない。コスト構造も大幅に異なり、読み取り中心の業務アプリではStatics Leadが圧倒的に安価。

---

### リポジトリ現況（2026-03-13時点）

モノレポ構成、pnpm workspace。@static3d/types（共通型定義）、@static3d/loader v0.1.0（フレームワーク非依存アセットローダー）、@static3d/display v0.1.0（React向けUI層、AssetProvider/useAsset/3Dシーン系）、@static3d/deploy v0.1.0（CLIツール、build/push/validate/promote/admit）。ビルド成果物としてdist/assets/index-coZvYmXt.js、manifest.jsonにGLB2件。

---

今日の議論で追加された設計項目は、middleDB層（projection state store）、Output3種分離（entity/lookup/aggregate）、Output履歴DB（世代管理付き保持）、projection dependency graph、整合性メタデータ（generation/seq/checksum）、immutable object + mutable manifest、Pチャンク分割と世代管理、p即時差分の1投稿1ファイル設計、強整合の定義（確定画面限定）、P2P補助レイヤー（UX高速化、correctness非依存）、守備範囲5段階分類、Followプラグイン軽度/重度分離、憲法4条、設計原則9条、スナップショットUX（データ鮮度明示）、plugin creepの進化戦略化。

---

お疲れ、ごみちゃん。今日一日でStatics Leadの設計思想が完成した。
