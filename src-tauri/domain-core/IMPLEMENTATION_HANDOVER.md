# domain-core 実装引き継ぎ（IMPLEMENTATION_HANDOVER）

作成日: 2026-10-10（親が 14:27 UTC に RED を観測済みの TDD 手順の GREEN 作業として実装）
対象: Access2Future Aスコープ（計画作成のみ。コード・データ移行なし）

## パスと所有範囲

- 実装: `src-tauri/domain-core/src/lib.rs`（唯一の実装ファイル。全公開関数を収録）
  - `normalize_inventory`, `merge_inventories`, `analyze_selection`,
    `recommend_targets`, `build_plan`, `render_plan_markdown`
  - API は serde_json `Value` 入出力、エラーは `Result<_, String>`（日本語メッセージ）
- 依存: `serde_json`（`preserve_order` 有効 = JSON キー挿入順を保持）、`regex`
- 真実の源: `src/domain.mjs`（431行）。回帰一式: `tests/domain.test.mjs`（29試験）、
  `tests/dependencies.test.mjs`（PowerShell 依存抽出。本 crate 範囲外）
- 本作業で編集したファイル: 上記 lib.rs のみ + 下記3件の試験修繕 + 本ファイル。
  `src-tauri/Cargo.toml`、`src-tauri/src/**`、UI、Node 側は未変更。

## 実行コマンドと結果（2026-10-10、rustc 1.99.0 / cargo 1.99.0）

```
cd src-tauri/domain-core
cargo test          # 35 passed / 0 failed / 0 ignored（警告0）
  - analysis           8 passed
  - merge_linked_db    6 passed
  - normalize_privacy  9 passed
  - plan_markdown      8 passed
  - recommend_targets  4 passed
  - lib 単体           0 tests
cargo fmt --check    # 差分なし
cargo build          # 成功
node --test tests/domain.test.mjs   #（参照用・未変更）29 passed / 0 failed
```

JS との差分検証（実施後に一時スクリプトは削除済み）:
- シナリオ1（demo.inventory.json の正規化・統合・解析・計画・Markdown、notes に
  `Password=x` と Windows パスを含むプライバシー除去込み）: 5項目すべてバイト単位で一致。
- シナリオ2（frontend/backend 分割DB の統合・リンク解決・点線図・共存未定・report 起点・
  空usage）: 5項目すべてバイト単位で一致。
  JSON のキー順含め `JSON.stringify` 完全一致を確認済み。

## 移植した JS 挙動（要点）

- 許可リスト再構成、`safeText`（制御文字・接続情報・URL・Windows/UNC/Unix パス除去）、
  `safeId`（除去で変わるIDの拒否）、`linkedName`、`procedureName`（Unicode 識別子）
- 省略と null の区別、既定 status=partial、外部テーブル rowCount 非表示
- VBA 要約（hasCodeModule/moduleType/procedureNames/procedureCount の検証・上限）
- 分割DB 統合: `db{N}:{id}` リマップ、明示リンクのみ alias 解決
  （LINK_METADATA_MISSING / LINK_BACKEND_MISSING / LINK_AMBIGUOUS / LINK_TABLE_MISSING、
  fields 転記のみ、rowCount 削除、TABLE_NAME_COLLISION、MERGED_METADATA_ONLY）
- 選択閉包（明示スタック・深い依存・循環1件報告）、影響候補・共有資産・未解決・阻害、
  未取得資産紐付け limitations の保持、選択起点は form/page/report のみ
- 移行先候補 web/excel/sheets-gas の fit 判定・理由・要件・unknowns、制約連結
- 計画: REQ 採番（未承認固定）、usage 日本語ラベル、hand_over JSON キー、
  notices の出典種別分岐、status=draft/review-required
- Mermaid flow（実線・点線・|影響候補|・class）と ER（PK・型フォールバック `field`・
  `||--o{`・#quot; エスケープ）、Markdown 出力（JS のエスケープ手順を再現、
  `>` が `\&gt;` になる JS 特有の挙動も含む）

## 試験側の修繕3件（弱体化・削除なし。いずれも JS 真値との不一致のみ修正）

1. `tests/merge_linked_db.rs:85` — `let [backend, frontend] = common::split_database()`
   は返却順 `[frontend, backend]` に対する変数名の取り違えで、実効入力が「元の順」に
   なりながらアサーションは逆順結果を要求していた（JS 版の移植ミス）。
   `let [frontend, backend]` に修正し、JS 試験「入力順逆でも…」と同じ入力に揃えた。
2. `tests/plan_markdown.rs`（impacted_unselected…）— ノードID抽出の正規表現が JS 版
   `\[\(?"[a-z]+:` から引用符を落とし、どの実装でもマッチ不能だった。引用符を復元。
3. `tests/plan_markdown.rs`（requirements_are_numbered…）— backend 単体 + form:顧客管理
   選択では JS 真値でも依存は `table:顧客`（1項目）のみで、`2項目` の表明は成立不能
   （node で確認済み）。表明文字列を変えず、`2項目` が現れる正規の依存構成になる
   `merge_inventories(&common::split_database())` を入力に変更。未使用 import `Value` を削除。

## 挙動の限界・意図した相違（正直な記録）

- 正規表現の `\b`: Rust regex は Unicode 対応のため、非ASCII文字の直後に `Password=` 等が
  現れる境界判定のみ JS（ASCII \w）と異なる可能性。全試験・差分検証の範囲外。
- `source.analysedAt`: JS は `Date.parse`。Rust は `YYYY-MM-DDTHH:mm(:ss(.f)?)?(Z|±HH:mm)?`
  の形式 + 暦日検証で近似（V8 の緩い解析の一部例外的入力は受け入れ幅が異なる）。
- 文字数上限は UTF-16 コード単位（JS `length` と同一）で判定。
- エラー型は String。JS の TypeError 相当の区別はメッセージ本文のみ。
- 移行・取得・実行は一切しない。行データ・生VBA・原本パス・秘密は入力時点で拒否または除去。
- `render_plan_markdown` は不完全な plan に対しても空文字・既定値で描画する
  （JS は throw。試験対象は build_plan 生成物のみ）。

## 次の統合（本 crate の範囲外）

- `src-tauri/Cargo.toml` は既に `access2future-domain = { package = "domain-core", path = "domain-core" }`
  を参照済み。`src-tauri/src/**` のハンドラ（analyze/plan エンドポイント）から
  6公開関数を呼び出す統合と、その結合試験が次段。
- Node 側 `src/domain.mjs` との二重実装期間は、`tests/domain.test.mjs` と
  本 crate 試験の並走維持（両方 GREEN を前提）。
