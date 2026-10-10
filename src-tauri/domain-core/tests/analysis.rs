mod common;
use common::{asset, asset_x, inventory, inventory_x, sel};
use domain_core::{analyze_selection, build_plan, normalize_inventory};
use serde_json::{json, Value};

#[test]
fn selection_closure_and_impacts_on_shared_db_are_distinguished() {
    let result = analyze_selection(
        &normalize_inventory(&common::shared_inventory()).unwrap(),
        &sel(&["form:受注"]),
    )
    .unwrap();
    assert_eq!(
        common::strings_of(&result, "selectedIds"),
        vec!["form:受注"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "dependencyIds")),
        vec!["query:受注", "table:受注", "table:顧客"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "impactedIds")),
        vec!["form:顧客", "report:請求"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "sharedIds")),
        vec!["table:受注", "table:顧客"]
    );
    assert!(common::strings_of(&result, "evidence")
        .iter()
        .any(|v| v.contains("form:受注")));
    assert!(!common::strings_of(&result, "selectedIds").contains(&"form:顧客".to_string()));
}

#[test]
fn direct_reference_to_selected_form_counts_as_impact() {
    let input = normalize_inventory(&inventory(&[
        asset("form:Child", "form", &[]),
        asset("form:Parent", "form", &["form:Child"]),
        asset("query:Mid", "query", &["form:Child"]),
        asset("form:Indirect", "form", &["query:Mid"]),
    ]))
    .unwrap();
    let result = analyze_selection(&input, &sel(&["form:Child"])).unwrap();
    assert_eq!(
        common::strings_of(&result, "selectedIds"),
        vec!["form:Child"]
    );
    assert!(common::strings_of(&result, "dependencyIds").is_empty());
    assert_eq!(
        common::sorted(common::strings_of(&result, "impactedIds")),
        vec!["form:Indirect", "form:Parent"]
    );
    assert_eq!(common::strings_of(&result, "sharedIds"), vec!["form:Child"]);
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:Child"], "usage": common::solo_usage() }),
    )
    .unwrap();
    assert!(plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["title"] == json!("未選択機能への影響を確認する")));
    assert_eq!(plan["analysis"]["selectedIds"], json!(["form:Child"]));
}

#[test]
fn cycles_broken_refs_dynamic_refs_and_unsupported_do_not_silently_pass() {
    let input = inventory(&[
        asset("form:受注", "form", &["query:A", "page:旧画面"]),
        asset_x(
            "query:A",
            "query",
            &["query:B", "table:未取得"],
            json!({ "status": "partial", "issues": [{ "code": "DYNAMIC_REFERENCE", "message": "実行時のテーブル名を確認" }] }),
        ),
        asset("query:B", "query", &["query:A"]),
        asset_x(
            "page:旧画面",
            "page",
            &[],
            json!({ "status": "unsupported" }),
        ),
    ]);
    let result =
        analyze_selection(&normalize_inventory(&input).unwrap(), &sel(&["form:受注"])).unwrap();
    let unresolved = result["unresolved"].as_array().unwrap();
    assert!(unresolved
        .iter()
        .any(|i| i["code"] == json!("MISSING_REFERENCE")
            && i["referenceId"] == json!("table:未取得")));
    assert!(common::has_issue(&result, "unresolved", "CYCLE"));
    assert!(common::has_issue(
        &result,
        "unresolved",
        "DYNAMIC_REFERENCE"
    ));
    assert!(result["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["assetId"] == json!("page:旧画面")));
    assert_eq!(
        common::strings_of(&result, "selectedIds"),
        vec!["form:受注"]
    );
}

#[test]
fn deep_dependencies_do_not_recurse_and_report_one_cycle() {
    let mut assets = vec![asset("form:A", "form", &["query:0"])];
    for index in 0..2000 {
        let next = if index == 1999 { 0 } else { index + 1 };
        assets.push(asset(
            &format!("query:{index}"),
            "query",
            &[&format!("query:{next}")],
        ));
    }
    let result = analyze_selection(
        &normalize_inventory(&inventory(&assets)).unwrap(),
        &sel(&["form:A"]),
    )
    .unwrap();
    assert_eq!(common::strings_of(&result, "dependencyIds").len(), 2000);
    assert_eq!(
        result["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["code"] == json!("CYCLE"))
            .count(),
        1
    );
}

#[test]
fn limitations_bound_to_unfetched_assets_are_kept() {
    let input = normalize_inventory(&inventory_x(
        &[asset("form:A", "form", &[])],
        json!({ "limitations": [{ "code": "PROTECTED", "assetId": "module:未取得", "message": "保護された定義は未取得です" }] }),
    ))
    .unwrap();
    assert!(
        analyze_selection(&input, &sel(&["form:A"])).unwrap()["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["assetId"] == json!("module:未取得"))
    );
}

#[test]
fn report_root_closure_and_impacts() {
    let input = normalize_inventory(&inventory(&[
        asset_x(
            "report:請求",
            "report",
            &["query:請求", "macro:印刷"],
            json!({ "status": "partial", "issues": [{ "code": "STATIC_DEFINITION_ONLY", "message": "保存定義のみ" }] }),
        ),
        asset("query:請求", "query", &["table:受注"]),
        asset("table:受注", "table", &[]),
        asset_x("macro:印刷", "macro", &[], json!({ "status": "unsupported" })),
        asset("form:受注", "form", &["table:受注"]),
        asset_x("page:旧画面", "page", &["table:受注"], json!({ "status": "unsupported" })),
        asset("report:月次", "report", &["table:顧客"]),
        asset("form:顧客", "form", &["table:顧客"]),
        asset("table:顧客", "table", &[]),
        asset("module:共通", "module", &[]),
        asset("external:外部", "external", &[]),
    ]))
    .unwrap();
    let result = analyze_selection(&input, &sel(&["report:請求"])).unwrap();
    assert_eq!(
        common::strings_of(&result, "selectedIds"),
        vec!["report:請求"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "dependencyIds")),
        vec!["macro:印刷", "query:請求", "table:受注"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "impactedIds")),
        vec!["form:受注", "page:旧画面"]
    );
    assert_eq!(
        common::sorted(common::strings_of(&result, "sharedIds")),
        vec!["table:受注"]
    );
    let selected = common::strings_of(&result, "selectedIds");
    for id in [
        "form:受注",
        "page:旧画面",
        "report:月次",
        "form:顧客",
        "macro:印刷",
        "module:共通",
        "external:外部",
    ] {
        assert!(!selected.contains(&id.to_string()), "{id}");
    }
    let impacted = common::strings_of(&result, "impactedIds");
    assert!(!impacted.contains(&"report:月次".to_string()));
    assert!(!impacted.contains(&"form:顧客".to_string()));
    assert!(result["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["assetId"] == json!("macro:印刷") && i["code"] == json!("UNSUPPORTED")));
    assert!(!result["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["assetId"] == json!("page:旧画面")));
    assert!(result["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["assetId"] == json!("report:請求") && i["code"] == json!("PARTIAL")));
    assert!(common::has_issue(
        &result,
        "unresolved",
        "STATIC_DEFINITION_ONLY"
    ));
    assert!(common::strings_of(&result, "evidence")
        .iter()
        .any(|v| v.contains("report:請求") && v.contains("自動追加していません")));
}

#[test]
fn selection_root_accepts_only_form_page_report() {
    let input = normalize_inventory(&inventory(&[
        asset("form:受注", "form", &["table:受注"]),
        asset_x(
            "page:旧画面",
            "page",
            &[],
            json!({ "status": "unsupported" }),
        ),
        asset("report:請求", "report", &["table:受注"]),
        asset("table:受注", "table", &[]),
        asset("query:受注", "query", &["table:受注"]),
        asset("macro:印刷", "macro", &[]),
        asset("module:共通", "module", &[]),
        asset("external:外部", "external", &[]),
    ]))
    .unwrap();
    assert_eq!(
        analyze_selection(&input, &sel(&["form:受注"])).unwrap()["selectedIds"],
        json!(["form:受注"])
    );
    let page = analyze_selection(&input, &sel(&["page:旧画面"])).unwrap();
    assert_eq!(page["selectedIds"], json!(["page:旧画面"]));
    assert!(page["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["assetId"] == json!("page:旧画面") && i["code"] == json!("UNSUPPORTED")));
    assert!(!common::strings_of(&page, "selectedIds").contains(&"form:受注".to_string()));
    assert!(!common::strings_of(&page, "selectedIds").contains(&"report:請求".to_string()));
    for id in [
        "table:受注",
        "query:受注",
        "macro:印刷",
        "module:共通",
        "external:外部",
    ] {
        let err = analyze_selection(&input, &sel(&[id])).unwrap_err();
        assert!(err.contains("帳票"), "{id}: {err}");
    }
    assert!(analyze_selection(&input, &sel(&[]))
        .unwrap_err()
        .contains("帳票"));
}

#[test]
fn analysis_result_shape_is_complete() {
    let result: Value = analyze_selection(
        &normalize_inventory(&common::shared_inventory()).unwrap(),
        &sel(&["form:受注"]),
    )
    .unwrap();
    for key in [
        "selectedIds",
        "dependencyIds",
        "impactedIds",
        "sharedIds",
        "unresolved",
        "blockers",
        "evidence",
    ] {
        assert!(result[key].as_array().is_some(), "{key}");
    }
}
