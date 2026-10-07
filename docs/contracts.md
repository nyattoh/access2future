# 実装契約

今回の実装対象はA（移行計画を作る版）。移行先アプリの生成・移行実行をしない。全体構造を解析し、フォーム・ページ・レポートを選び、利用形態を確認して、依存・未選択機能への影響・要件・移行と検証の計画を示す。

## Inventory version 1

```json
{
  "schemaVersion": 1,
  "source": { "name": "example.accdb", "kind": "access", "accessVersion": "16", "analysedAt": "ISO date", "fingerprint": "SHA256" },
  "assets": [{ "id": "form:受注入力", "kind": "form", "name": "受注入力", "caption": "受注入力", "dependsOn": ["query:受注一覧"], "status": "supported", "issues": [], "hasCodeModule": true, "moduleType": "class", "procedureCount": 2, "procedureNames": ["Form_Load", "Recalculate"] }],
  "relations": [{ "from": "table:受注", "to": "table:顧客", "fields": [{ "from": "顧客ID", "to": "ID" }], "enforced": true }],
  "limitations": [{ "code": "DYNAMIC_REFERENCE", "message": "動的参照は確認が必要", "assetId": "module:業務処理" }]
}
```

- source.kind: `access` / `inventory` / `synthetic`。合成サンプルを実Access解析として表示しない。
- asset.kind: `table` / `query` / `form` / `page` / `report` / `macro` / `module` / `external`。直接選択対象は `form` / `page` / `report`。`page` はAccess Data Access Pagesを指し、レガシー非対応として阻害事項を示す。
- VBA要約は任意の `hasCodeModule:boolean`、`moduleType:standard|class`、`procedureCount:0..100000`、最大1000件の重複しない `procedureNames:string[]` を持つ。各手続き名は安全な識別子で255文字以下。フォーム/レポートのコードビハインドは所有元のform/report assetに付属し、`moduleType=class`。標準／独立クラスモジュールは `kind=module` で `moduleType` に種別を示す。`moduleType` はコードモジュールの存在を示す。`hasCodeModule=false` では `moduleType`、空でない手続き名、0より大きい件数を指定できない。非対応ページにはVBA要約を付けない。生ソースはこのInventoryに含めない。
- asset.status: `supported` / `partial` / `unsupported`。取得・解析の状態であり、移行再現の保証ではない。
- asset.issues: `{code,message}` の配列。dependsOnは対象idの配列。未取得参照を黙って消さない。
- table.fields: `{name,dataType,required,isPrimaryKey}` の配列。local tableのrowCountは、計数した場合だけ数値として示す。外部テーブルの行は取得しない。
- Accessリンクのtable.linkedDatabaseNameは接続先のbasenameだけ、linkedTableNameはSourceTableName。複数資料の統合時は、この明示されたリンクだけを解決し、同名だけでテーブルを同一視しない。
- raw VBA・接続文字列・認証情報・元の絶対ファイルパスはAPIの応答やエクスポートへ含めない。
- SQLや保存定義を解析する場合も、文字列にある接続秘密を除去し、動的参照を不確実性として保持する。
- VBAはAccessの保存定義から一時作業領域へソースを抽出し、手続き宣言、明示的/定義名照合による呼出し候補、静的SQL/Accessオブジェクト参照、`CreateObject`、`Declare Lib` を検出する。コメントや通常の文字列中のオブジェクト名は参照として扱わない。生ソースはAPI応答・ログ・Inventoryへ含めず、抽出後に一時領域を削除する。VBA、フォーム、帳票、マクロは実行しない。
- 動的参照、連結SQL、同名手続き、未取得先は `DYNAMIC_REFERENCE`、`AMBIGUOUS_PROCEDURE_REFERENCE`、`UNRESOLVED_PROCEDURE_REFERENCE` 等で未解決を残す。これは静的な依存候補であり、実行時依存・条件分岐の完全性は保証しない。フォーム/レポートのコードビハインドはForm/Report.Module経由でHasModuleを確認してから読む。
- `DataAccessPages` はAccessの旧ページ資産で、常に `unsupported`。コンテナー列挙自体に失敗した場合は `DATA_ACCESS_PAGES_UNAVAILABLE` を返し、0件と取得失敗を区別する。`Scripts` コンテナーに列挙される保存マクロ以外の埋め込み/データマクロは取得範囲外。
- 外部データ参照、暗号化、破損、保護されたオブジェクト、レガシーのページなどの取得不能を部分対応・非対応として明示する。
- 同名の直接参照がフォームとレポートなど複数候補に一致する場合は `AMBIGUOUS_REFERENCE` をissueとして残す。必須のAccessオブジェクトコンテナーを列挙できなかった場合は `OBJECT_CONTAINER_UNAVAILABLE` limitationを返す。名前の全文文字列照合に伴う誤検出可能性は残る。

## Domain module: src/domain.mjs

- `normalizeInventory(input)`: 入力を検証し、既知の安全なフィールドを持つInventoryを返す。不正な版・重複id・不正な型を拒否する。
- `analyzeSelection(inventory, selectedIds)`: `{selectedIds,dependencyIds,impactedIds,sharedIds,unresolved,blockers,evidence}` を返す。dependencyIdsは選択対象以外の依存資産。impactedIdsは共有資産に依存する未選択のform/page/report。循環で停止せず、選択を勝手に広げない。
- `recommendTargets(inventory, usage, analysis)`: `{id,label,fit,reasons,requirements,unknowns}` の配列。idは `web` / `excel` / `sheets-gas`、fitは `recommended` / `conditional` / `not-recommended`。相対的な計画支援の判断であり、未実証の適合保証をしない。
- `buildPlan(inventory, options)`: optionsは `{selectedIds,usage,targetId,notes}`。`{version,status,source,analysis,usage,target,requirements,migrationSteps,validationSteps,risks,unresolved,notices,diagrams,hand_over}` を返す。diagramsは `{flow,er}` のMermaid文字列で、取得済みのdependsOnとrelationsだけから作る（依存範囲にテーブルがなければerは空文字）。statusは `draft` / `review-required`。requirementsは `{id,title,description,evidence,verification,confirmed}`、confirmedは自動でtrueにしない。
- `renderPlanMarkdown(plan)`: 人が確認できる計画を返す。出典・未解決事項・合成/実Accessの区別を含む。
- `mergeInventories(inventories)`: フロント/バックなど複数の構造資料を、sourceごとのIDに分離して統合する。linkedDatabaseNameとlinkedTableNameが一致する場合だけリンクを解決し、未提供・曖昧はlimitationとして保持。source.kindはinventory。統合しただけで実取得や移行再現を保証しない。

usage: `{users,concurrentEditing,location,offlineRequired,permissions,coexistence}`。
usersは `solo` / `team`、locationは `device` / `lan` / `remote`、permissionsは `same` / `roles`、coexistenceは `undecided` / `keep-source` / `shared-store` / `replace-scope`。

## Access importer: src/access-import.mjs

- `importAccess(filePath, options = {})`: Promise<Inventory>。渡されたコピーのメタデータだけを読み、原本や外部データを更新しない。成功・部分対応・非対応を区別する。
- `getAccessCapabilities()`: Promise<{available:boolean,reason?:string}>。COMが使えるかを検査し、利用不能を成功として扱わない。
- 公開HTTPは任意のローカルパスを受け付けず、アップロードのランダムな一時パスだけを渡す。
- 外部/ユーザDBをAccessのアクティブなDBとして開かず、安全な一時DBへの定義の取り込みを候補とする。ユーザの起動処理・フォーム・マクロ・VBAは実行しない。検証できない場合はフォーム等の取得を保留して部分対応とする。
- child_processは引数配列を使い、shellを使用しない。タイムアウトと自己所有のCOM・一時ファイルの終了処理を行う。

## Local server: src/server.mjs

- localhostにbind。APIに送られたAccessファイルを外部APIに送信しない。実データ・秘密・生定義をログにしない。
- `GET /api/health`: version、Access解析の利用可能性。
- `GET /api/demo`: 明示的にsyntheticとしたInventory。
- `POST /api/import`: raw binary、ファイル名は `X-File-Name`（percent encoded）。.mdb / .accdb、最大64MiB、署名を検査し、Access importerへ渡す。取込みは直列、解析上限は15分（900,000ms。serverがimportAccessのtimeoutMsへ渡す）。取得不能・時間切れを成功と扱わない。時間切れはHTTP 504 `ACCESS_TIMEOUT`、メッセージに「15分」を明記し、結果は作成しない。
- `POST /api/import/cancel`: 進行中の取込みを中止し、worker終了と一時ファイルの削除後に `{cancelled:boolean}` を返す。実行中の処理がない場合はfalse。中止・削除を確認できない場合はエラー。
- `POST /api/import`の任意ヘッダー`X-Import-Id`: UUID。画面はファイルごとに新しい値を作る。`GET /api/import/progress?requestId=<UUID>`は、そのIDの`{requestId,state,phase,completed,total}`だけを返す。stateはwaiting/running/completed/cancelled/failed、phaseはpreparing/enumerating/analysing/finishing。totalは列挙中null、その後は固定した資産数。名前・SQL・接続・パスは含めない。ID違い・件数逆行・不正値・取消後のcallbackは破棄する。分母は列挙後に固定した資産総数、分子は確認処理を終えた資産数で、割合に時間は使わない。全資産の確認が済むとphase=finishing（completed==total）になり、その後のクリーンアップ・HTTP応答が続く間もstate=runningのまま。失敗・時間切れ時はstate=failedとし、最後に確認できたphase/completed/totalを消さない（totalがnullなら未確定のまま）。画面は失敗後に最終スナップショットを1回取得し、エラー近くに残す。
- `POST /api/inventory`: Inventory JSONを検証して返す。入力上限を設ける。
- `POST /api/plan`: `{inventory,selectedIds,usage,targetId,notes}`。`{plan,markdown}` を返す。
- `POST /api/merge`: `{inventories}`。ユーザーが明示的に取り込んだ構造資料だけを統合する。参照先のローカルパスに自動アクセスしない。
- `public/` のファイルと `src/domain.mjs` の公開モジュールだけを配信。パス遡及を拒否する。
- エラーを日本語で表示できる形にし、不正入力や取得不能を明示する。

## Ownership

- 主担当: package.json、src/server.mjs、tests/server.test.mjs、docs（designを除く）、.gitignore、統合・ブラウザ検証。
- Access担当: src/access-import.mjs、scripts/export-access.ps1、scripts/create-sample.ps1、tests/access-import.test.mjs、tests/native-smoke.ps1。
- Domain担当: src/domain.mjs、tests/domain.test.mjs、samples/demo.inventory.json。
- luna担当: docs/design/内の3端末のPNG画像とデザイン仕様だけ。画像前提のpaletteは別途渡す。
- UI担当: public/だけ。design画像と契約を基にレスポンシブで実装。

すべての担当は同じ作業ツリーで作業する。他者の編集を戻さず、所有範囲外を変更しない。TDDは先に受入条件に対応するテストを実行して失敗を確認し、その後に実装して成功を確認する。失敗を記録し、成功だけの事後テストをTDDと呼ばない。

## UIの受入シナリオ

- 画像3枚とdocs/design/DESIGN.mdを基に、1440x900、1024x1366、390x844で作業の順序と配置を維持する。
- 初期画面: 見出し「データベースを解析」、「ファイル1」「ファイル2」のAccess/JSON入力、「解析開始」「合成サンプルを試す」。どちらか1つだけでも可。選択・選び直し・削除では解析せず、結果を無効化する。明示的な開始後に個別に解析し、2つの場合はapi/merge。実ファイル・JSON・合成を区別する。処理中は中止以外の変更・移動を止める。
- ステップ2: 見出し「フォーム・ページを選択」。検索・種別フィルタ、実際の資産から作るcheckbox（accessible nameはcaption/name）、選択数、未選択の影響説明、「利用形態へ」。未選択では次へをdisabledにする。表/行は本物のDOMで操作可能にする。
- ステップ3: 見出し「利用形態を確認」。利用人数radio「一人で使う」「複数人で使う」、同時編集checkbox「同時に入力・編集する」、利用場所radio「同じPCから」「社内の複数PCから」「社外・遠隔から」、checkbox「オフラインでも使う」、権限select（label「権限」、same/roles）、共存select（label「既存Accessとの共存」、契約enum）。「影響を確認」。人数・場所・通信は別の軸として扱う。
- ステップ4: 見出し「依存関係と影響を確認」。選択/依存/共有/未選択への影響/未取得・非対応を別々に表示する。移行先radio「Webアプリ」「Excel」「Googleスプレッドシート + GAS」と理由・条件を表示。「計画を作成」。ユーザーの明示選択を保持する。
- ステップ5: 見出し「移行計画」。status草案・要件未承認、要件/根拠/移行手順/検証手順/未解決/リスクを人が読める形で表示し、「Markdownを保存」「JSONを保存」を実行できる。テキストをHTMLとして挿入しない。
- 全工程に戻る/工程ナビがあり、入力と選択を保持して変更後は計画を再生成する。loading/error/empty状態、keyboard/focus、touch target、固定footerによる本文隠れを確認する。
