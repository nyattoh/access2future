"""User journeys and responsive checks using synthetic material only."""
import json
import re
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:7331"
ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "artifacts" / "screenshots"
OUTPUT.mkdir(parents=True, exist_ok=True)
VIEWPORTS = [("desktop", 1440, 900), ("tablet", 1024, 1366), ("phone", 390, 844)]
results = []

with sync_playwright() as runner:
    browser = runner.chromium.launch(headless=True)
    try:
        for name, width, height in VIEWPORTS:
            context = browser.new_context(viewport={"width": width, "height": height}, accept_downloads=True)
            page = context.new_page()
            page.set_default_timeout(8000)
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.goto(BASE, wait_until="networkidle")
            expect(page.get_by_role("heading", name="データベースを解析", exact=True)).to_be_visible()
            page.get_by_role("button", name="合成サンプルを試す", exact=True).click()
            expect(page.get_by_role("heading", name="フォーム・ページ・帳票を選択", exact=True)).to_be_visible()
            expect(page.get_by_text("合成サンプル", exact=True).first).to_be_visible()
            expect(page.get_by_role("button", name="利用形態へ", exact=True)).to_be_disabled()
            page.get_by_role("checkbox", name="受注入力", exact=True).check()
            page.get_by_role("checkbox", name="顧客管理", exact=True).check()
            assert page.locator("input[type=checkbox]:checked").count() == 2, "Selection expanded silently"
            assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"), "Horizontal overflow"
            page.screenshot(path=str(OUTPUT / f"{name}-selection.png"), full_page=True)
            last_choice = page.get_by_role("checkbox").last
            last_choice.scroll_into_view_if_needed()
            bounds = last_choice.bounding_box()
            footer = page.locator(".actions").bounding_box()
            assert bounds["y"] + bounds["height"] < footer["y"], "Choice obscured by fixed actions"
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            impact_end = page.locator("aside .muted").last.bounding_box()
            assert impact_end["y"] + impact_end["height"] < footer["y"], "Impact end obscured by fixed actions"
            page.get_by_role("button", name="利用形態へ", exact=True).click()
            expect(page.get_by_role("heading", name="利用形態を確認", exact=True)).to_be_visible()
            page.get_by_role("radio", name="複数人で使う", exact=True).check()
            page.get_by_role("checkbox", name="同時に入力・編集する", exact=True).check()
            page.get_by_role("radio", name="社外・遠隔から", exact=True).check()
            page.get_by_label("権限", exact=True).select_option("roles")
            page.get_by_label("既存Accessとの共存", exact=True).select_option("undecided")
            page.get_by_role("button", name="影響を確認", exact=True).click()
            expect(page.get_by_role("heading", name="依存関係と影響を確認", exact=True)).to_be_visible()
            expect(page.get_by_text(re.compile("請求書")).first).to_be_visible()
            page.get_by_role("radio", name="Webアプリ", exact=True).check()
            page.get_by_role("button", name="計画を作成", exact=True).click()
            expect(page.get_by_role("heading", name="移行計画", exact=True)).to_be_visible()
            expect(page.get_by_text(re.compile("未承認")).first).to_be_visible()
            assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"), "Plan overflow"
            page.evaluate("""() => {
              window.__savePickerCalls = [];
              window.__savedFiles = [];
              window.showSaveFilePicker = async (options) => {
                window.__savePickerCalls.push(options);
                return {
                  createWritable: async () => ({
                    write: async (content) => window.__savedFiles.push({ name: options.suggestedName, content }),
                    close: async () => {},
                  }),
                };
              };
            }""")
            page.get_by_role("button", name="Markdownを保存", exact=True).click()
            expect(page.get_by_role("status")).to_have_text("access-migration-plan.md を保存しました。")
            assert page.evaluate("window.__savePickerCalls.length") == 1, "Markdown did not open the save picker"
            saved_markdown = page.evaluate("window.__savedFiles[0].content")
            assert "受注入力" in saved_markdown and "未承認" in saved_markdown and "合成" in saved_markdown
            assert page.evaluate("window.__savePickerCalls[0].suggestedName") == "access-migration-plan.md"

            page.get_by_role("button", name="JSONを保存", exact=True).click()
            expect(page.get_by_role("status")).to_have_text("access-migration-plan.json を保存しました。")
            assert page.evaluate("window.__savePickerCalls.length") == 2, "JSON did not open the save picker"
            saved_plan = json.loads(page.evaluate("window.__savedFiles[1].content"))
            assert saved_plan["usage"]["users"] == "team" and saved_plan["usage"]["location"] == "remote"
            assert page.evaluate("window.__savePickerCalls[1].suggestedName") == "access-migration-plan.json"

            page.evaluate("window.showSaveFilePicker = undefined")
            with page.expect_download() as event:
                page.get_by_role("button", name="Markdownを保存", exact=True).click()
            expect(page.get_by_role("status")).to_contain_text("ダウンロード")
            download = event.value
            destination = OUTPUT / f"{name}-plan.md"
            download.save_as(destination)
            content = destination.read_text(encoding="utf-8-sig")
            assert "受注入力" in content and "未承認" in content and "合成" in content
            with page.expect_download() as event:
                page.get_by_role("button", name="JSONを保存", exact=True).click()
            expect(page.get_by_role("status")).to_contain_text("ダウンロード")
            destination = OUTPUT / f"{name}-plan.json"
            event.value.save_as(destination)
            plan = json.loads(destination.read_text(encoding="utf-8-sig"))
            assert plan["usage"]["users"] == "team" and plan["usage"]["location"] == "remote"
            assert plan["status"] == "review-required"
            assert all(item["confirmed"] is False for item in plan["requirements"])
            assert len(plan["analysis"]["selectedIds"]) == 2
            assert any(item["code"] == "COEXISTENCE_UNDECIDED" for item in plan["unresolved"])
            page.screenshot(path=str(OUTPUT / f"{name}-plan.png"), full_page=True)
            assert not errors, errors
            results.append({"viewport": name, "width": width, "height": height, "flow": "passed", "console_errors": len(errors)})
            context.close()
    finally:
        browser.close()

print(json.dumps({"results": results, "source": "synthetic_only", "real_device": False}, ensure_ascii=False))
