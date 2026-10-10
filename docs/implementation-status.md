# Aの実装・検証結果

## 最新状態 2026-10-11

Rust/TauriのWindows release exeに、業務フロー候補図とER図をアプリ内表示する変更を追加した。ローカル同梱のMermaid Tiny 12.1.0を使用し、Node 64件、Rust 50件合格、native専用Rust試験1件除外、npm audit 0件を確認した。Synthetic browser flowとNSIS install smokeも合格。計画画面はNode Web adapterで3サイズを目視確認した。インストールしたTauri appから静的資産とAPIを確認し、NSIS uninstallも成功した。Computer Use native pipeが利用できず、Tauri WebView2の実ウィンドウ画像は未取得。Commit `7220960`はpush済みで、[Draft PR #3](https://github.com/nyattoh/access2future/pull/3)はGrok独立reviewを待つ。公開releaseは未作成。

### 2026-10-11 独立レビュー後の修正・再検証

Grok 4.7 (`runtimeMode=auto`) のレビューは、起動時のmain window二重作成、cancel waiterの完了取りこぼし、一時ファイル削除失敗の見落としをP1として指摘した。起動時の問題はレビュー回収前にconfig側のwindow定義を削除して解消し、実exeを起動して再確認した。cancel waiterは消費される結果ではなくwatch完了状態を共有し、終了済みjobの結果を複数の待機者から読めるようにした。削除エラーはworker結果に優先して伝え、HTTP cancelに解析IDを付けて一致した解析だけを中止し、終了後に記録された後処理エラーも同じIDで報告する。画面は実際の`cancelled`値に応じて完了通知を分ける。

TDD記録: `.local/autonomous-run-20261011/evidence/cancel-id-red.log`では後処理エラーのID照合テストが失敗し、修正後の`cancel-id-green.log`はpass。`cancel-waiters-green.log`は複数waiter、`cancel-cleanup-api-red.log` / `cancel-cleanup-api-green.log`は削除失敗応答を検証。`cancel-mismatched-id-green.log`はID不一致時にactive jobを止めないことを確認。`npm test` 63/63、Rust workspace 49 passed / 1 ignored、format check、合成native Access smoke (6 assets, source hash unchanged)、ブラウザー合成シナリオ7項目を再実行してpass。release exe再build後、起動・loopback token (tokenなし403)・11資産demo・帳票選択plan・Mermaid flowchart/ERを確認。NSIS installerとnative Tauriの画面キャプチャ、実Access資料の確認は未実施。

Grokのcompleted二次レビューは、上記P1の解消を確認したうえでUIのP2を2件指摘した。import IDなしのJSON/merge中止通知を正しくし、ID付きAccess cancelがエラーになった場合は最終進捗snapshotを1回取得し検証して表示するよう修正した。回帰 browser testは9 scenario pass。別個のTDD RED/GREEN記録は`.local/autonomous-run-20261011/evidence/ui-cancel-json-notice-red.log` / `ui-cancel-json-notice-green.log`、`ui-cancel-progress-red.log` / `ui-cancel-progress-green.log`。JSON/merge中止通知と、進捗4/10から最終8/10・後処理への更新を合成データで確認した。`npm-p2-final.log`は63/63。Access COMの実取消・ファイル削除失敗は実Accessで誘発していない。

## 2026-10-07 利用者承認後の追加範囲

利用者は帳票の直接選択、「ページ」= Access Data Access Pages、取得可能なVBAソースの抽出・静的依存解析・リファクタリング計画の提示を決定した。これらはA（移行計画を作る版）の範囲内である。過去節にある「未決/未対応」はその時点の履歴として読み、現在の決定は本節とREADME、`docs/handover.md` §13を参照する。

合成行入りのローカルダミーDBから、75保存定義と50 VBAソースを抽出。33 form code-behind、15 report code-behind、2 standalone standard module、standalone class module0。416 procedure declarations、80 call candidates（52 uniquely resolved, 26 ambiguous, 2 unresolved）、17 resolved cross-asset edges、15/50 files with dynamic refs。object reference candidates 147（130 resolved、5 external、17 unresolved）。これは静的候補の集計で、実行時挙動や網羅性の証明ではない。生ソースはGit除外領域へ保存し、API outputへ含めない。

Mermaid図とリファクタリング提案はGit除外 `.local/private-import/vba-review/analysis-20261007-final-v4/`。明示的な6 DAO relationで結ばれたentityは36 table中8件に限られる。図に現れない表も業務上無関係とはいえない。業務名はスキーマ/オブジェクト名からの仮説。

TDDでREDを確認後、pi / `zai/glm-5.3` / thinking highにdomain/UI変更を委任し、親が統合確認した。境界REDでプロシージャ名255文字と空選択エラー文言の不一致も検出・修正。最終 `npm test` は59/59、`npm run test:native` は6資産・1 relation・5 limitations・progress 6/6・source hash unchanged、PowerShell依存テスト内39 assertions pass。`node --check` を3ファイルで確認。T3 preview DOMで合成サンプルから帳票を選択し、依存と未選択フォーム/ページへの影響を確認した。T3 snapshot/resizeはautomation timeoutとなり今回の3 viewport画像は未取得。実Access原本・行データは今回開いていない。詳細と実task IDは `docs/routing.md`、`docs/handover.md` §13を参照。

初期確認日: 2026-10-05。対象は利用者が選んだ **A：移行計画を作れる版**。後続のTDD実装指示に基づき実装した。Bの移行先コード生成・データ移行・切替は対象外。2026-10-07の更新は本書先頭の追記を参照。

## 利用できる機能

Node.js標準ライブラリのローカルHTTPサーバー、ブラウザのJavaScript / CSS、Windows PowerShell / Access COMで構成する。実行時のnpm依存パッケージはない。サーバーは127.0.0.1だけで待ち受ける。

1. AccessのDB全体のメタデータ、JSON資料、汎用の合成サンプルを取り込む。任意の別バックエンド資料も取り込める。
2. 明示されたリンク先ファイル名と表名が一致する資料を統合する。同名の表だけを根拠に結合しない。不明・未提供・曖昧な参照を残す。
3. 利用者が移行したいフォーム・ページを選び、依存する資産・共有資産・未選択機能への影響を確認する。レポートは解析・影響候補に含まれるが直接選べない。この制限を変更するかは未決。選択は自動で増えない。
4. 人数・同時編集・PC / LAN / 遠隔・オフライン・権限・既存Accessとの共存を別々に質問する。
5. Webアプリ、Excel、Googleスプレッドシート + GASの候補と理由を提示する。要件候補、移行・検証手順、未確認事項とリスクをMarkdown / JSONで保存する。

取込み画面は「ファイル1」「ファイル2」と表示し、どちらか1つでも始められる。選択だけでは処理せず「解析開始」で解析する。選び直し・選択の削除・解析中止ができ、処理中は中止以外の変更・移動を止める。選び直すと古い解析・選択・計画を無効化する。

Access解析中は、確認処理が済んだ資産数 / 総資産数を実測し、ファイルごとの割合をバーで示す。総数を数える間は段階名を表示する。割合は時間や取得成功率ではない。100%は資産確認の完了であり、後処理と原本保全の確認はPOST応答が成功するまで待つ。中止待ちは最後の実測値を保持し、完了・失敗・中止後にバーを消す。JSON資料は入力された資産数を基準に検証する。

要件のconfirmedはすべてfalse。草案の状態はdraftまたはreview-requiredであり、業務担当者の承認や移行可否を自動で確定しない。推奨はローカルの条件規則による目安で、各移行先への互換性を実証したものではない。製品実行時に外部AI APIを呼ばない。

主なコードは [domain.mjs](../src/domain.mjs)、[access-import.mjs](../src/access-import.mjs)、[server.mjs](../src/server.mjs)、[export-access.ps1](../scripts/export-access.ps1)、[public](../public/index.html)。契約は [contracts.md](contracts.md)、手順は [usage.md](usage.md)。

## 確認済みの検証

| 検証 | 結果 | 範囲 |
|---|---|---|
| npm test | 52 / 52合格（2026-10-06の参照曖昧性・列挙失敗表示修正後も合格） | ドメイン20、HTTP18、importer13、PowerShell依存抽出1（内部24チェック） |
| npm run test:native | 合格 | 合成Access実ファイルから6資産・1リレーションを取得、入力SHA256不変 |
| tests/browser-flow.py | 3サイズ合格 | 1440×900、1024×1366、390×844で選択からMarkdown / JSON保存、ページエラー0、横はみ出しなし |
| 固定操作欄の回帰 | RED後GREEN | 最終候補と影響説明末尾が下端に隠れる反例を修正し3サイズで確認 |
| 取込み画面の追加回帰 | 3サイズで6シナリオ合格 | 自動解析なし、選び直し・削除、両欄の単独解析、2ファイル統合、処理中の操作阻止、中止・遅い応答・再試行。T3 previewで合成JSONのみ使用 |
| 私的ダミーのnative抽出 | 合格 | フロント127資産（38フォーム・34クエリ）、バック18表、リレーション6 |
| 私的ダミーの画面取込み | 合格 | Accessファイル2つのHTTP取込みは両方200、統合した38候補から1つを選択、利用形態・計画JSON保存、原本とダミーのハッシュ不変 |
| 私的ダミーの構造・行 | 合格 | 計36表・新規合成行36、型・Required差異0、FK違反0 |
| 強制タイムアウト | 合格 | ACCESS_TIMEOUT、作成したAccessプロセス残存0、既存プロセス保全、入力ハッシュ不変 |
| 解析中止 | 合格 | worker終了・入力不変・一時フォルダー削除。起動後25 / 150 / 500msの中止でもAccess残存0、既存Accessを保全 |
| 実測進捗・native HTTP | 合格 | ダミーのフロント127資産で76回の実測スナップショット、途中経過を取得し最後127 / 127。バックは18 / 18。原本・入力ハッシュ不変 |
| 実測中のHTTP中止 | 合格 | 12 / 127でcancelledを確認。中止後の件数固定、アップロード削除、入力ハッシュ不変 |
| 実測表示の画面回帰 | 合格 | 合成protocolで20→60%、ID違い・逆行・取消・古いJSON読取りを確認。T3 preview使用 |
| 独立コアレビュー（初回） | 再確認範囲の残存P0/P1/P2なし | 初回4指摘をpi GLMが修正しCodexが再確認。実機・全画面の保証ではない |
| 今回の初期画面・計画画面監査 | 3 viewportを保存し確認 | 1440×900、1024×768、390×844。合成サンプルで全5工程を表示。レポートの影響表示と選択後のキーボードフォーカスを回帰確認。画面証拠はGit除外の `.local/autonomous-run-20261005/`。キーボード/支援技術の全確認ではない |

TDDと実際のprovider / model / options / task ID / 終状態は [routing.md](routing.md)。UI参照・比較と未検証の端末条件は [design-qa.md](../design-qa.md)。

## Accessと私的資料の扱い

渡されたAccessファイルを読み取り専用の一時コピーとしてDAOで開く。ユーザDBをAccessの現在のDBとして開かず、空の作業DBへ定義だけを取り込む。フォーム、クエリ、マクロ、VBAは実行しない。外部リンクの行・スキーマは取得しない。アップロードは最大64MiB、JSONは最大8MiB、取込みは直列。一時ファイルは成功応答前に削除し、タイムアウト時はPID・起動時刻・プロセス名が一致した自己所有のAccessだけを終了する。

Access資産の取得範囲: importer は Reports / Scripts / Modules を列挙し、保存定義を空の作業DBへ構造のみ取り込んで `SaveAsText` し、定義全文に対して資産名を文字列照合する。VBA ASTやプロシージャ間呼び出しを解析せず、コードは実行しない。`module` は標準/クラスの区別を保持しない。フォーム/レポートのコードビハインドが保存定義にどこまで含まれるか、独立クラスモジュールの全件取得、埋め込み/データマクロは未検証。現行 `page` は `DataAccessPages` の暫定対応であり、必ず非対応とする。レポートの直接選択はUI・domainで未対応、方針は未決。

Named `SourceObject` が複数のフォーム/レポート候補に一致する場合は `AMBIGUOUS_REFERENCE` issueを追加し、`Add-DirectReference`単独では候補を一つに決めない。ただし定義全文の先行文字列照合が複数候補をdependsOnへ追加する可能性は残るため、影響の完全性は保証しない。Forms/Reports/Scripts/Modulesコンテナーの列挙失敗は `OBJECT_CONTAINER_UNAVAILABLE` としてInventory limitationsへ記録する。legacy `DataAccessPages` の列挙例外は一律に無視するため、コンテナー不在と予期しない失敗は区別されず、ページ不在も証明しない。この診断経路は合成PowerShellテストで確認し、実Accessでの権限/破損エラーは未試験。

利用者指定のMDB / ACCDBはローカルに保全し、テーブルの実行をせずに構造から新規ダミーDBを生成した。実データの行は読み出していない。ファイル全体のコピー・ハッシュ計算と、テーブル行の抽出は区別する。デフォルト式37件は元の処理を実行しないため抑止した。Requiredの初期差異22件は修正し、再比較で0を確認した。

**初回の失敗:** TransferDatabaseの入力にした保全コピー2つのハッシュが変わった。利用者指定の原本2つは不変。失敗・変更済みコピーを保持し、その後は新規の読み取り専用作業コピーを使用した。最終監査はoriginalsMatchManifest=true、dummyHashesUnchanged=true、preservationCopiesMatchManifest=falseであり、最初から全コピーが不変だったとは扱わない。

私的DBのnative抽出では109資産が部分取得、クエリ定義1件が取得不能。依存関係が完全であるという意味ではない。監査・DB・取得資料・失敗ログはGit除外の `.local/private-import/`。私的な構造や定義も匿名の公開資料とは扱わず、外部AI・画像生成・別providerへ送信していない。私的DBの画面取込みは初版時の結果であり、追加の入力改善では合成JSONによる画面回帰と、ダミーAccessによるnative中止を別々に確認した。

native試験ではnpm配下でGet-FileHashが解決されないハーネスの失敗があり、.NET SHA256に変更して合格した。画面取込みハーネスでもCSPに反する文字列evalと、その修正後の待機タイムアウトを記録した。HTTP取込みの上限は当初180秒だったが、私的な30.8MB・128資産のMDBが75/128定義の転送中（180,783ms、確認済み74/128、転送中央値7.6秒/定義）で時間切れとなった。native・server双方を15分（900,000ms）に変更し、serverから上限をimportAccessへ渡す。時間切れはACCESS_TIMEOUT（504、「15分」表記）で、server snapshotはstate=failedのまま最後の実測件数を保持し、画面はファイル選択を残して件数・割合・段階をエラー近くに表示する。分母は固定した資産総数、割合は確認済み件数のみから算出する。

再試行では同じMDBを読み取り専用・定義のみでnative解析し、552,458msで128/128資産が完了した。途中スナップショット76回、元ファイルのSHA256は実行前後で一致、テーブル行は読み取っていない。Node 52件、Windows / Accessの合成サンプル試験6件も合格。実MDBの完走は直接のnative解析で確認した。HTTP経由のフロント127/127・バックエンド18/18、画面の時間切れと最後の実測件数はダミーDBまたは合成protocolで回帰確認済み。実MDBをブラウザーからアップロードする再試行は未実施。

## 再現と未確認

```sh
npm start
npm test
npm run test:native
python -X utf8 tests/browser-flow.py
```

ブラウザ試験はPython / Playwright / Chromiumを別途使用する。native試験はWindows / Accessが必要。私的ダミーの生成・監査はローカルmanifestに依存し、公開サンプルでは再現できない。

VBA・動的SQLの完全解釈、Access全バージョン・暗号化・破損・全特殊型・第三者コントロール、Data Access Pagesの再現、実機スマホ／タブレット、実ユーザによる使いやすさ評価は未確認。外部SQL INを含む定義では同名ローカル表への誤結合を避けるため、他の表・クエリの静的結合も保留する。

依存候補の文字列照合では、コメント・文字列内の資産名が誤検出になる可能性がある。同名 `SourceObject` は曖昧性issueで明示するが、先行文字列照合で複数候補を依存に載せる場合は残る。必須のオブジェクトコンテナー列挙失敗はlimitationで示すが、Access上で権限・破損を模した失敗は未試験。legacy `DataAccessPages` は列挙例外を無視するため、存在有無と取得失敗の判別は残課題。

実装は未コミット。公開・PR・push・ライセンス設定・本番の移行先選定や移行は実施していない。

追加UI回帰の再現: ローカルアプリをT3 previewで開き、tests/upload-flow.browser.jsのuploadFlowRegression関数をpreview_evaluateで実行する。各シナリオは同関数に記載。既存browser-flow.pyは合成サンプルから計画保存までの試験として維持する。T3 previewは今回利用可能になったため、入力改善の画面確認はT3側で実行した。

実測進捗の画面回帰はtests/measured-progress.browser.js。画面の応答は合成protocol、native HTTPの実測は私的ダミーとして区別する。監査はGit除外の.local/measured-progress/に保持。実測段階で総数・完了件数以外の名前・SQL・パスを通知しない。時間切れ・失敗後の件数保持（15分、74/128＝57%、総数未確定時は割合なし、再試行可）も同ファイルの合成protocolで確認する。


## 2026-10-07 128/128後処理エラー

同じ失敗を合成frontend dummyで再現。PowerShellはexit0、JSON出力は有効。Node側のdomain検証で hasCodeModule=false と procedureNames=[] / procedureCount=0 が併存し拒否されることを特定した。コードモジュールなしのフォーム/帳票にこの空要約が24件含まれる。空の手続き一覧・0件は許可し、正の要約値やmoduleTypeは引き続き拒否するよう変更。

新規importer回帰テストRED→GREEN。修正後の同一dummified frontend importは127 assets / 6 relations / 220 limitations / no-code assets24 / 82 progress events / finishing127/127で成功。npm test 60/60、native synthetic smoke 6/6、PowerShell dependency39 assertions pass。実ユーザーMDBの再試行は未実施。修正版ローカルアプリは稼働中、HTTP200。
## 2026-10-11 Rust/Tauri Windows continuation

製品ゴールはAの移行計画版。Windows向けの専用WebView2画面、Rustの解析・計画API、既存PowerShell 5.1 Access COM helperを組み合わせた。行データの移行、生成コード、公開、push、PRは行っていない。私的Access原本とその行データは開かず、合成サンプルだけで試験した。

`src-tauri/domain-core/` にNode領域処理から移植した6関数を統合。`src-tauri/src/http.rs` は起動ごとのtoken、Host/Origin、CSP、JSON入力の検査を持ち、`src-tauri/src/access.rs` は既存PowerShell helperを呼び出す。Web版はNode adapter経由で同じJSON APIを使う。

TDDで2つの実装欠陥を再現して修正した。Rust正規表現のUnicode単語境界が、日本語の直後にある接続情報やURLを除去しないケースをテストし、JSと同じASCII境界にした。PowerShell helperが返す`ACCESS_EXTRACTION_FAILED`をRust側が`ACCESS_UNAVAILABLE`へ誤変換していたため、worker error codeを分類して保つようにした。別の実行試験ではTauri設定とsetup hookの両方が`main`ウィンドウを作り、起動exeがexit 101となった。設定から重複定義を除いた。

最終確認は`npm test` 63/63、`cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`成功、`cargo test --manifest-path src-tauri/Cargo.toml --workspace` 43 passed / 1 ignored、`npm run test:native`合成Access 6 assets / 1 relation / 5 limitations、Rust native smoke合格、入力SHA256不変。`python -X utf8 tests/browser-flow.py http://127.0.0.1:7332`は1440x900、1024x1366、390x844で合格しJavaScript console error 0だった。

`npm run desktop:build -- --no-bundle`で`src-tauri/target/release/access2future.exe`を構築し、重複窓修正後の起動・loopback APIを試験した。`GET /`は200、tokenのない`/api/health`は403、token付きhealthはlocal、合成demoは11 assets、report選択のplan応答に`hand_over`、Mermaid flowchart、ER図が含まれた。API smokeは合成JSONのみで実行し、プロセスは試験後に終了した。

NSISインストーラーは未生成。環境に`makensis.exe`がなく、新しいソフトやビルドツールを入れない指示に従った。WebView2ランタイムとWindows/Rust/MSVCは`cargo tauri info`で確認した。no-bundle exeは開発元リポジトリからのlocal smoke用で、配布可能なインストーラーではない。

画面証拠はGit除外`.local/autonomous-run-20261011/evidence/`に保存した。Node Web版を合成データで操作して7 PNGを開いて確認し、91.4秒のMP4を作成して再生時間と再生位置の進行を確認した。3 viewport browser-flowは物理端末での試験ではない。Tauri native window自体の画像はComputer Use native pipeが利用不能だったため取得できず、native UIの目視検証は未実施。代わりにTauri release exeの起動とlocal APIを確認した。

独立コードレビューはGrok 4.7で進行中。CursorのGemini 3.8 FlashとGemini 3.7 Flashはprovider turn開始前に失敗した。Claude/OpenRouterのAPI契約レビューは以前に402 insufficient creditsで失敗しており、課金回避の再試行はしていない。Antigravityのlive catalogueには実行可能modelがなかった。bench ledgerは`no_t3_models_in_ledger`を返し、live T3 catalogueからGrokを選択した。実task IDは`docs/routing.md`。

今回の連続実行は8時間に達していない。scheduler list呼び出しはservice errorとなり、新しいtimerは作成していない。前回の8時間予約は2026-10-06記録でdisabled確認済み。Grok taskの最終状態、独立指摘、NSIS環境の用意、Tauri native screenshotが次の作業である。

### 2026-10-11 継続作業の最終更新

上記記録後、Grok 4.7の二次レビュー task `node:delegated-task:command%3Amcp%3A9e7f6cad-0500-43e8-9962-72f9dbaf6eee%3Adelegate-task%3Aaccess2future-grok-cancellation-fix-review-20261011-r2` がcompletedとなった。既出P1の解消を確認し、新たに示したUI P2 2件を修正した。Access import IDのないJSON/merge中止ではローカル中止を知らせ、ID付きcancelがcleanup errorを返した場合は同じIDの進捗snapshotを取得し、検証して件数・段階を残す。

TDD RED/GREENは `.local/autonomous-run-20261011/evidence/ui-cancel-json-notice-red.log` / `ui-cancel-json-notice-green.log` と `ui-cancel-progress-red.log` / `ui-cancel-progress-green.log`。T3 browser regression 9 scenario pass: JSONとmergeの中止通知、Access ID伝播、cleanup error後の進捗4/10から8/10 (80%)・後処理表示を合成モックで確認した。Node test 63/63、`node --check`対象JS両方pass。実Accessでのcleanup失敗、NSIS installer、Tauriネイティブ画面撮影は未実施。最終review結果は `.local/autonomous-run-20261011/evidence/grok-round2-final.md`。

UI修正を含めて `npm run desktop:build -- --no-bundle` を再実行し、release exeの起動後に `/app.mjs` が修正後の中止表示・ID付き最終進捗取得コードを配信すること、API tokenなし403、health local、合成demo 11 assets、帳票planとMermaid図を確認した。証拠は `tauri-build-p2-final.log` と `tauri-exe-live-smoke-p2-final.json`。
合成ブラウザー画面でcleanup failureと最終進捗8/10 (80%)を同時表示したPNG `evidence/07-20261011-cleanup-failure-progress.png` を撮影し、開いて内容を目視確認した。これはNode Web adapterの画面であり、Tauriネイティブ画面ではない。

### 2026-10-11 Mermaid図の画面内表示とWindows配布試験

ユーザーの追加要件により、移行計画画面に業務フロー候補図とER図を表示する。MarkdownとJSONにも従来どおりMermaidソースを保存する。図の描画には外部通信を使わず、MITライセンスのMermaid Tiny 12.1.0を`public/vendor/mermaid.tiny.js`として同梱する。フロー図とER図に必要な機能を含み、KaTeX数式機能は含まない。

TDDでは画面の合成計画にSVGがないためbrowser testが失敗したことを確認し、ライブラリ描画、CSP対応Blob画像、altテキスト、エラー時のソース表示を追加した。全Mermaidを含む最初の候補bundleにはKaTeX 0.16.47が残っており、npmのoverridesでは既成bundleを更新できなかったため不採用とした。Mermaid Tinyへ変更後に描画とNSIS buildをやり直した。

確認結果: `npm test`は64件すべて合格。`npm audit`は脆弱性0件。`cargo fmt --check`は合格。Rust workspaceは50件合格し、native専用1件を除外した。Rustの静的資産試験で`app.mjs`と`plan-diagrams.mjs`のJavaScript MIME型を確認した。T3 browser flowは合成計画の10項目が合格。renderer testは描画、altテキスト、無効なソースの表示を確認した。1280×800のPC、820×1180のタブレット、390×844のスマホで2つの図を表示し、文書幅はviewport幅を超えなかった。画像は `evidence/11-20261011-mermaid-tiny-desktop.png`、`12-20261011-mermaid-tiny-tablet.png`、`13-20261011-mermaid-tiny-phone.png`。

NSIS current-user installerは3,767,255 bytes、SHA256 `828AA7F37224D05354ECE7F44FB3BDDF04368F5B81411DA366B3DFEBE2C62421`。合成データだけを使って一時ディレクトリに導入し、実行ファイルがloopback page、vendor asset、tokenなし403、11資産demo、flow/ER planとMarkdown Mermaid fencesを返すことを確認した。Uninstaller exit code 0、install directory removed。アプリはこの試験後に停止し、永続インストールはない。Install reportは `.local/autonomous-run-20261011/evidence/nsis-mermaid-tiny-smoke.json`。

Computer Useの実ウィンドウ撮影は `Computer Use native pipe is unavailable: failed to connect native pipe: 指定されたファイルが見つかりません。 (os error 2)` で不可。表示の目視確認はNode Web adapterの同一frontendで行い、WebView2 native画面の画像確認とは区別する。署名・公開release、Git push、Draft PRはこの記録時点で未実施。ユーザーはpushとPR/releaseを希望し、pushとDraft PR準備を許可済み。案件規則に沿い、PRをDraftで作成してGitHub review待ちとする。

### 2026-10-11 Apache-2.0選択

利用者は初回公開版のプロジェクトライセンスとしてApache-2.0を選択した。公式Apache FoundationのLICENSE-2.0本文を`LICENSE`へ配置し、README、npm package、Rust application/domain-core manifestsへ`Apache-2.0`識別子を記載。Mermaid TinyのMIT条件を含む依存・同梱ソフトウェアの個別ライセンス条件は維持する。公開releaseはDraft PR #3のreview/merge後まで保留し、Windows installerは未署名のまま。
