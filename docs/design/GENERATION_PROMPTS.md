# Image generation prompts

Each final reference was generated in a separate call through the built-in `image_gen` tool. The requested viewport is a composition target; the tool selected the actual PNG resolution documented in `DESIGN.md`. No CLI or `OPENAI_API_KEY` was used. The built-in tool did not report a model identifier. All content is generic synthetic sample data.

## Desktop

```text
Use case: ui-mockup
Asset type: desktop product UI reference image for a Japanese Access migration planning tool, one application screen, target viewport 1440x900.
Primary request: Show step 2 of a five-step workflow, “フォーム・ページを選択”. The tool analyses an Access database inventory, lets the user select forms/pages, checks usage in the next step, then explains dependencies and impacts and prepares a migration and verification plan. This is planning only; do not generate code or execute migration.
Scene/backdrop: A realistic, clean desktop browser application on a warm off-white canvas, straight-on screenshot, no browser chrome.
Subject: A calm, precise business application used by Japanese office staff. Dense but comfortably readable. Flat, functional UI with visible table rows, thin dividers, square or barely rounded corners, generous spacing, 44px minimum controls.
Composition/framing: At 1440x900. Three-column app: narrow left workflow sidebar; central selection list taking most width; right impact explanation pane. Simple top header with product name “Access移行計画”, a small discreet source label “合成サンプル”, and step count/progress. Sidebar has five numbered stages in order: “データベースを解析”, “フォーム・ページを選択” (active), “利用形態を確認”, “影響を確認”, “計画を作成”. Central panel title exactly “フォーム・ページを選択”; short explanatory sentence; compact search field and category filters. Checkable rows with names, type, dependency note, and checkbox: selected “受注入力”, “顧客管理”, “在庫照会”, “商品一覧”, and unselected “請求一覧”, “月次レポート”. Show selection summary “4件選択中”. Right pane titled “選択による影響”, with a clear shared-data explanation: “受注入力と顧客管理は共有テーブル「顧客」「受注」を参照します。” and unselected impact warning “未選択の「請求一覧」は受注テーブルを共有します。請求業務の移行範囲を確認してください。” Include compact neutral linked-item lines and avoid charts/metrics. Fixed bottom action area with back link “戻る” and clear red primary button “利用形態へ”.
Style/medium: high-fidelity production SaaS screenshot, flat conventional Japanese business software, clear hierarchy and legibility, not a wireframe.
Lighting/mood: neutral, quiet, trustworthy, workmanlike.
Color palette: exact restrained colors: Access red #B12334 only for selected/active emphasis and primary action, text #232323, secondary text #6B6B6B, borders #E5E1DC, page background #F7F5F2, white surfaces #FFFFFF.
Materials/textures: entirely clean digital UI, no shadows except none or imperceptible.
Text (verbatim): “Access移行計画”, “合成サンプル”, “データベースを解析”, “フォーム・ページを選択”, “利用形態を確認”, “影響を確認”, “計画を作成”, “受注入力”, “顧客管理”, “在庫照会”, “商品一覧”, “請求一覧”, “月次レポート”, “4件選択中”, “選択による影響”, “受注入力と顧客管理は共有テーブル「顧客」「受注」を参照します。”, “未選択の「請求一覧」は受注テーブルを共有します。請求業務の移行範囲を確認してください。”, “戻る”, “利用形態へ”.
Constraints: Screen only, no people, all values are generic synthetic sample data, no real personal data, no real measured figures, no code generation or migration claims. Keep typography readable and crisp; preserve Japanese UI structure.
Avoid: gradients, glassmorphism, floating card clusters, glow, purple, sparkles, mascots, decorative AI logos, giant hero areas, legacy Access imitation, skeuomorphism, meaningless KPIs, graphs, marketing illustrations, tiny unreadable text, ornate corners, excessive shadows, extra columns.
```

## Tablet

```text
Use case: ui-mockup. Generate a high-fidelity, flat desktop-class tablet app screenshot for a Japanese Access migration planning tool, target viewport 1024x1366 portrait. Show the same screen and state as this specification. Layout: compact top app header “Access移行計画” and discreet “合成サンプル” label; compact five-step progress navigation with “フォーム・ページを選択” active and these stages: “データベースを解析”, “フォーム・ページを選択”, “利用形態を確認”, “影響を確認”, “計画を作成”. Main title “フォーム・ページを選択” and short helper copy, search field, then central checkable rows laid out in one broad list with a narrower impact section underneath or clearly below the list. Rows: selected “受注入力”, “顧客管理”, “在庫照会”, “商品一覧”; unselected “請求一覧”, “月次レポート”; summary “4件選択中”. Impact section “選択による影響” with exact meaning/text “受注入力と顧客管理は共有テーブル「顧客」「受注」を参照します。” and “未選択の「請求一覧」は受注テーブルを共有します。請求業務の移行範囲を確認してください。” Bottom fixed/safe action area: “戻る” and primary “利用形態へ”. Styling: ordinary polished, usable Japanese office business software, warm near-white #F7F5F2 background, white surface #FFFFFF, Access red #B12334 used sparingly for active step, checked controls, and primary CTA, main ink #232323, muted text #6B6B6B, dividers #E5E1DC. Clearly delineated list rows and text hierarchy, generous spacing, crisp Japanese system sans, controls minimum 44px, square or barely rounded corners, no shadows. Important: synthetic example only; no real personal details, no real measured values, no claims of migration execution or code output. This is a planning tool. Avoid gradients, glassmorphism, floating card clusters, glow, purple, sparkles, mascots, decorative AI marks, oversized hero, legacy Access imitation, skeuomorphism, meaningless metrics or charts, marketing art, extra unrelated features, excessive roundness, unreadably tiny labels. Straight-on UI screenshot, no browser chrome, no people.
```

## Phone (final revision)

```text
Use case: ui-mockup. Produce a high-fidelity screenshot of a Japanese mobile business web app at a 390x844 viewport. This is one screen in an Access migration planning tool, step 2 of 5.
Composition: strictly single-column mobile layout with one vertical, full-width selection list and a bottom sticky action area. Top app bar says “Access移行計画” and “合成サンプル”. Step navigation MUST be compact and mobile-native: show only “2 / 5” plus current stage “フォーム・ページを選択” and a thin short progress indicator. Do not show the five stage names across the screen; no horizontal five-item nav.
Main content: heading “フォーム・ページを選択”, short one-line explanation, full-width search field, “4件選択中”. Six clear full-width selectable list rows with checkbox and label: checked “受注入力”, “顧客管理”, “在庫照会”, “商品一覧”; unchecked “請求一覧”, “月次レポート”. Each row is divided by a thin rule; rows have generous height and at least 44px tap targets. Below the list, a simple full-width impact area “選択による影響”, with concise readable lines: “受注入力と顧客管理は共有テーブル「顧客」「受注」を参照します。” and “未選択の「請求一覧」は受注テーブルを共有します。請求業務の移行範囲を確認してください。” Bottom sticky action area has a secondary “戻る” button and a wide primary “利用形態へ” button.
Style: restrained Japanese office tool, crisp Japanese system sans typography, flat white surfaces on #F7F5F2, ink #232323, muted #6B6B6B, thin borders #E5E1DC, restrained Access red #B12334 only for checked boxes and main CTA. No decorative imagery.
Must be a genuine portrait mobile layout with readable, realistic hierarchy. Generic synthetic sample values only; never present real measured data or personal information. Plan-only product: no code generation or migration execution.
Avoid: desktop layouts, columns, side panes, five-column/five-stage navigation, tiny horizontal labels, gradients, glass, floating card clusters, glow, purple, sparkles, mascots, decorative AI logos, huge hero, legacy Access imitation, skeuomorphism, charts, meaningless metrics, marketing illustrations, tiny type, crowded controls, extra features.
```
