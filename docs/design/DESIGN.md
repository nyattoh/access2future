# Access migration planning UI reference

This document defines the visual handover for the Japanese planning-only product. It covers selecting forms and pages after database inventory analysis. The product explains likely shared dependencies and impacts, then asks for usage details and prepares a migration and verification plan; it does not generate application code or perform migration.

All screen content in the references is generic synthetic sample content. It is not a measurement, a real Access analysis, or personal information. Use the screenshots as responsive composition references; implement all text and controls as real DOM content.

## Reference images

| Layout | Image | Intended viewport | Generated PNG resolution |
| --- | --- | ---: | ---: |
| Desktop | [desktop-reference.png](desktop-reference.png) | 1440 × 900 | 1586 × 992 |
| Tablet | [tablet-reference.png](tablet-reference.png) | 1024 × 1366 | 1086 × 1449 |
| Phone | [phone-reference.png](phone-reference.png) | 390 × 844 | 853 × 1844 |

The image-generation tool returned its own pixel dimensions rather than the requested viewport dimensions. Do not infer CSS pixel size from PNG resolution.

## Layout and responsive behaviour

- Desktop (1200px and wider): use a narrow left five-step workflow navigation, a central selection list, and a right impact pane. Keep a stable bottom action area with “戻る” and “利用形態へ”.
- Tablet (768px–1199px): compact the workflow navigation into a horizontal five-step strip. Keep the list full-width and place the impact explanation below it. Keep both actions reachable at the bottom without covering content.
- Phone (under 768px): use a single vertical column. Show a compact “2 / 5” progress marker and the current step label; do not squeeze all five step names into a row. Use full-width, vertically stacked list rows and a safe-area-aware bottom action area. The impact explanation remains reachable by scrolling and must not be hidden behind the action area.
- At all sizes, keep the page title “移行対象を選択”, the synthetic sample notice, selection count, and the primary “利用形態へ” action visible in the flow. Search may narrow the list without changing the source inventory. The list covers forms, reports, and Access Data Access Pages; identify Data Access Pages as legacy and unavailable.

## Visual tokens

Use `palette.json` as the source of truth: accent `#B12334`, text `#232323`, muted text `#6B6B6B`, border `#E5E1DC`, background `#F7F5F2`, and surface `#FFFFFF`. Keep the red restrained for the active step, selected controls, and primary action. Do not introduce gradients, glow, shadows, purple, decorative AI marks, mascots, or floating card clusters.

Use a readable Japanese system sans stack such as `system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`. Use clear heading/body/secondary-text hierarchy; avoid overly light or condensed weights. A practical starting scale is 24–28px for the page heading, 16px for body and list labels, and 14px for secondary details, adjusted to fit the actual viewport.

Use a simple 4px spacing base: 4, 8, 12, 16, 24, 32, and 40px. Give sections and rows room to breathe. Prefer square corners or a very small radius, thin dividers, and no elevation. Every interactive target must be at least 44px high/wide.

## Selection list and workflow navigation

Keep list rows aligned and easy to scan. Each row has a real checkbox, the form/page name, its type, and a concise dependency or role hint when available. Use thin horizontal rules instead of separate floating cards. Show a persistent count such as “4件選択中”; selected state must be clear through both the checked control and its label context. Do not make selecting a form silently expand the selection.

Show these five workflow stages in this order: “データベースを解析”, “移行対象を選択”, “利用形態を確認”, “影響を確認”, “計画を作成”. Clearly distinguish completed, active, upcoming, and unavailable stages by text/state as well as colour. The current stage is step 2. Tablet can show all five labels; phone shows “2 / 5” and the current stage label.

The sample screen selects “受注入力”, “顧客管理”, “在庫照会”, and “商品一覧”. “請求一覧” and “月次レポート” remain unselected; the latter represents a selectable report. The impact explanation must call out that selected forms share “顧客” and “受注”, and that unselected functions also share assets. Present this as an impact to review, not a claim that migration will succeed or that the selection should change automatically.

## Interaction states

- Hover: use a subtle neutral row/background change; do not rely on hover alone to communicate selection.
- Focus: provide a clearly visible keyboard focus outline with sufficient contrast, using the accent where appropriate. Preserve native keyboard operation for checkboxes, search, and buttons.
- Disabled: keep the label legible, explain why an action is unavailable where useful, and prevent disabled controls from appearing active.
- Loading: retain the page structure and show a restrained inline status such as “読み込み中”; do not invent progress percentages. Keep unrelated navigation usable where safe.
- Error: show a plain Japanese explanation near the affected content and an actionable retry. Preserve the current selection when possible; never present incomplete analysis as complete.
- Empty: distinguish “no assets found” from “no search matches”. Explain the state and offer a clear search reset or next action. Do not show a fabricated zero-data chart.

## Implementation handover

Use the three PNGs for layout, hierarchy, density, and responsive behaviour only. Source exact interface copy from the product contract and render it as DOM text; generated Japanese glyphs and incidental sample details are not authoritative. Keep the “合成サンプル” label visible whenever demo inventory is displayed, and never describe synthetic names or dependencies as measured Access findings. Implement the tablet and phone reflow at the breakpoints above, keep the impact text reachable, and preserve the user’s explicit selection through search and viewport changes.

## Generation record

The final prompts used for these PNGs are recorded in [GENERATION_PROMPTS.md](GENERATION_PROMPTS.md). Images were produced separately with the built-in image-generation tool. The tool did not return a model identifier, so no model attribution is asserted here.
