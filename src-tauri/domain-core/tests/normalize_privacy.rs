mod common;
use common::{asset, asset_x, inventory, inventory_x, sel};
use domain_core::{analyze_selection, build_plan, normalize_inventory, render_plan_markdown};
use serde_json::{json, Value};

#[test]
fn metadata_allowlist_only_and_no_original_paths_or_secrets() {
    let input = inventory_x(
        &[asset_x(
            "form:A",
            "form",
            &[],
            json!({
                "rawVba": "TOP_SECRET", "connectionString": "Password=TOP_SECRET", "sql": "TOP_SECRET",
                "issues": [{ "code": "EXTERNAL", "message": "接続 Password=TOP_SECRET; Server=secret-host; C:\\private\\data.accdb" }]
            }),
        )],
        json!({
            "source": { "name": "C:\\private\\data.accdb", "kind": "access", "originalPath": "C:\\private\\data.accdb", "password": "TOP_SECRET" },
            "secrets": "TOP_SECRET"
        }),
    );
    let normalized = normalize_inventory(&input).unwrap();
    let output = normalized.to_string();
    assert_eq!(normalized["source"]["name"], json!("data.accdb"));
    assert!(!output.contains("TOP_SECRET"));
    assert!(!output.contains("private"));
    assert!(!output.contains("secret-host"));
    assert!(!output.contains("rawVba"));
    assert!(!output.contains("connectionString"));
    assert!(!output.contains("originalPath"));
    assert!(normalized["assets"][0]["issues"][0]["message"]
        .as_str()
        .unwrap()
        .contains("接続情報を除去"));
    assert_eq!(input["source"]["name"], json!("C:\\private\\data.accdb"));
}

#[test]
fn quoted_connection_secrets_are_redacted_with_the_whole_value() {
    for message in [
        "PWD=\"prefix\"\"SYNTHETIC_SECRET\";",
        "PWD='prefix''SYNTHETIC_SECRET';",
        "PWD=\"prefix\"SYNTHETIC_SECRET;",
        "Password=\"a;b\"SYNTHETIC_SECRET",
    ] {
        let normalized = normalize_inventory(&inventory(&[asset_x(
            "form:A",
            "form",
            &[],
            json!({ "issues": [{ "code": "CONNECTION", "message": message }] }),
        )]))
        .unwrap();
        let msg = normalized["assets"][0]["issues"][0]["message"]
            .as_str()
            .unwrap();
        assert!(msg.contains("接続情報を除去"), "{message}");
        assert!(!msg.contains("SYNTHETIC_SECRET"), "{message}");
    }
    assert!(normalize_inventory(&inventory(&[asset("form:PWD=\"a\"\"b\"", "form", &[])])).is_err());
    let input = normalize_inventory(&inventory(&[asset("form:A", "form", &[])])).unwrap();
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:A"], "usage": common::solo_usage(), "notes": "控え: PWD=\"prefix\"\"SYNTHETIC_SECRET\";" }),
    )
    .unwrap();
    let all = format!("{}{}", plan.to_string(), render_plan_markdown(&plan));
    assert!(!all.contains("SYNTHETIC_SECRET"));
}

#[test]
fn redacts_connection_secrets_and_urls_after_japanese_text() {
    let normalized = normalize_inventory(&inventory(&[asset_x(
        "form:A",
        "form",
        &[],
        json!({ "issues": [{ "code": "EXTERNAL", "message": "参照Password=TOP_SECRET; 接続https://example.invalid/path" }] }),
    )]))
    .unwrap();
    let message = normalized["assets"][0]["issues"][0]["message"]
        .as_str()
        .unwrap();
    assert!(message.contains("接続情報を除去"));
    assert!(message.contains("外部参照を除去"));
    assert!(!message.contains("TOP_SECRET"));
    assert!(!message.contains("example.invalid"));
}

#[test]
fn rejects_bad_versions_duplicates_types_and_selection_mistakes() {
    let bad_inputs = [
        json!(null),
        json!([]),
        inventory_x(&[], json!({ "schemaVersion": 2 })),
        inventory(&[asset("form:A", "form", &[]), asset("form:A", "form", &[])]),
        inventory(&[asset("form:A", "wrong", &[])]),
        inventory(&[asset_x(
            "form:A",
            "form",
            &[],
            json!({ "dependsOn": "table:A" }),
        )]),
        inventory(&[asset_x("table:A", "table", &[], json!({ "rowCount": -1 }))]),
        inventory(&[asset_x(
            "table:A",
            "table",
            &[],
            json!({ "fields": [{ "name": "ID", "required": "yes" }] }),
        )]),
    ];
    for input in &bad_inputs {
        assert!(normalize_inventory(input).is_err(), "{input}");
    }
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    assert!(analyze_selection(&input, &sel(&[])).is_err());
    assert!(analyze_selection(&input, &sel(&["form:存在しない"])).is_err());
    assert!(analyze_selection(&input, &sel(&["table:受注"])).is_err());
    assert!(build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "usage": { "users": 12 } })
    )
    .is_err());
    assert!(build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "usage": { "concurrentEditing": "true" } })
    )
    .is_err());
    assert!(build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "targetId": "unknown" })
    )
    .is_err());
}

#[test]
fn distinguishes_omitted_from_null_and_defaults_status_to_partial() {
    for extra in [
        json!({ "dependsOn": null }),
        json!({ "issues": null }),
        json!({ "status": null }),
    ] {
        assert!(
            normalize_inventory(&inventory(&[asset_x("form:A", "form", &[], extra.clone())]))
                .is_err(),
            "{extra}"
        );
    }
    assert!(normalize_inventory(&inventory_x(&[], json!({ "relations": null }))).is_err());
    assert!(normalize_inventory(&inventory_x(&[], json!({ "limitations": null }))).is_err());
    let input = inventory(&[json!({ "id": "form:A", "name": "A", "kind": "form" })]);
    let normalized = normalize_inventory(&input).unwrap();
    assert_eq!(normalized["assets"][0]["status"], json!("partial"));
    assert!(common::has_issue(
        &analyze_selection(&normalized, &sel(&["form:A"])).unwrap(),
        "unresolved",
        "PARTIAL"
    ));
}

#[test]
fn external_table_row_count_is_not_shown_as_loaded() {
    let input = normalize_inventory(&inventory(&[
        asset("form:A", "form", &["table:外部"]),
        asset_x(
            "table:外部",
            "table",
            &[],
            json!({ "isLinked": true, "rowCount": 999, "fields": [{ "name": "ID", "dataType": "Long" }] }),
        ),
    ]))
    .unwrap();
    assert!(input["assets"][0 + 1].get("rowCount").is_none());
    assert!(common::has_issue(
        &analyze_selection(&input, &sel(&["form:A"])).unwrap(),
        "unresolved",
        "EXTERNAL_REFERENCE"
    ));
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:A"], "usage": common::solo_usage() }),
    )
    .unwrap();
    assert!(render_plan_markdown(&plan).contains("未計数"));
}

#[test]
fn keeps_vba_summary_and_module_type_rules() {
    let input = inventory(&[
        asset_x(
            "form:受注",
            "form",
            &[],
            json!({ "hasCodeModule": true, "moduleType": "class", "procedureCount": 2, "procedureNames": ["Form_Load", "請求計算"] }),
        ),
        asset_x(
            "report:請求",
            "report",
            &[],
            json!({ "hasCodeModule": true, "moduleType": "class", "procedureCount": 1, "procedureNames": ["Report_Open"] }),
        ),
        asset_x(
            "module:標準",
            "module",
            &[],
            json!({ "hasCodeModule": true, "moduleType": "standard", "procedureCount": 1, "procedureNames": ["TaxRate"] }),
        ),
        asset_x(
            "module:クラス",
            "module",
            &[],
            json!({ "hasCodeModule": true, "moduleType": "class", "procedureCount": 0, "procedureNames": [] }),
        ),
        asset_x(
            "module:名前のみ",
            "module",
            &[],
            json!({ "procedureNames": ["OnlyName"] }),
        ),
        asset_x(
            "report:件数のみ",
            "report",
            &[],
            json!({ "hasCodeModule": true, "moduleType": "class", "procedureCount": 3 }),
        ),
        asset_x(
            "form:画面のみ",
            "form",
            &[],
            json!({ "hasCodeModule": false }),
        ),
    ]);
    let normalized = normalize_inventory(&input).unwrap();
    let find = |id: &str| {
        normalized["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == json!(id))
            .unwrap()
            .clone()
    };
    let form = find("form:受注");
    assert_eq!(form["kind"], json!("form"));
    assert_eq!(form["hasCodeModule"], json!(true));
    assert_eq!(form["moduleType"], json!("class"));
    assert_eq!(form["procedureCount"], json!(2));
    assert_eq!(
        common::strings_of(&form, "procedureNames"),
        vec!["Form_Load", "請求計算"]
    );
    let report = find("report:請求");
    assert_eq!(report["kind"], json!("report"));
    assert_eq!(report["moduleType"], json!("class"));
    assert_eq!(
        common::strings_of(&report, "procedureNames"),
        vec!["Report_Open"]
    );
    assert_eq!(find("module:標準")["moduleType"], json!("standard"));
    let class_module = find("module:クラス");
    assert_eq!(class_module["moduleType"], json!("class"));
    assert_eq!(
        common::strings_of(&class_module, "procedureNames"),
        Vec::<String>::new()
    );
    assert_eq!(class_module["procedureCount"], json!(0));
    let names_only = find("module:名前のみ");
    assert_eq!(
        common::strings_of(&names_only, "procedureNames"),
        vec!["OnlyName"]
    );
    assert!(names_only.get("procedureCount").is_none());
    assert!(names_only.get("moduleType").is_none());
    let count_only = find("report:件数のみ");
    assert_eq!(count_only["kind"], json!("report"));
    assert_eq!(count_only["moduleType"], json!("class"));
    assert_eq!(count_only["procedureCount"], json!(3));
    assert!(count_only.get("procedureNames").is_none());
    let bare = find("form:画面のみ");
    assert_eq!(bare["hasCodeModule"], json!(false));
    assert!(bare.get("moduleType").is_none());
    assert!(bare.get("procedureNames").is_none());
    assert!(!normalized.to_string().contains("Debug.Print"));
    let merged = domain_core::merge_inventories(&[input]).unwrap();
    let merged_form = merged["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == json!("受注"))
        .unwrap();
    assert_eq!(merged_form["kind"], json!("form"));
    assert_eq!(merged_form["hasCodeModule"], json!(true));
    assert_eq!(merged_form["moduleType"], json!("class"));
    assert_eq!(
        common::strings_of(merged_form, "procedureNames"),
        vec!["Form_Load", "請求計算"]
    );
}

#[test]
fn rejects_invalid_module_types_and_source_fragments() {
    let base = |extra: Value, kind: &str, id: &str| inventory(&[asset_x(id, kind, &[], extra)]);
    for extra in [
        json!({ "moduleType": "form" }),
        json!({ "moduleType": "report" }),
        json!({ "hasCodeModule": true, "moduleType": "standard" }),
        json!({ "hasCodeModule": "true" }),
        json!({ "procedureCount": -1 }),
        json!({ "procedureCount": 1.5 }),
        json!({ "procedureCount": "1" }),
        json!({ "procedureNames": "Form_Load" }),
        json!({ "procedureNames": ["Public Sub Form_Load()"] }),
        json!({ "procedureNames": ["Sub Form_Load()\n  Debug.Print 1\nEnd Sub"] }),
        json!({ "procedureNames": ["Form Load"] }),
        json!({ "procedureNames": ["1Load"] }),
        json!({ "procedureNames": ["PWD=\"secret\""] }),
        json!({ "procedureNames": ["C:\\private\\mod.bas"] }),
        json!({ "procedureNames": ["Form_Load", "Form_Load"] }),
        json!({ "procedureCount": 1, "procedureNames": ["Form_Load", "Extra"] }),
        json!({ "hasCodeModule": false, "moduleType": "class" }),
        json!({ "hasCodeModule": false, "procedureNames": ["Form_Load"] }),
        json!({ "hasCodeModule": false, "procedureCount": 1 }),
    ] {
        assert!(
            normalize_inventory(&base(extra.clone(), "form", "form:A")).is_err(),
            "{extra}"
        );
    }
    assert!(
        normalize_inventory(&base(json!({ "moduleType": "form" }), "report", "report:A")).is_err()
    );
    assert!(normalize_inventory(&base(
        json!({ "moduleType": "standard" }),
        "report",
        "report:A"
    ))
    .is_err());
    assert!(normalize_inventory(&base(
        json!({ "hasCodeModule": true, "moduleType": "form" }),
        "module",
        "module:A"
    ))
    .is_err());
    assert!(normalize_inventory(&inventory(&[asset_x(
        "table:A",
        "table",
        &[],
        json!({ "hasCodeModule": false })
    )]))
    .is_err());
    assert!(normalize_inventory(&inventory(&[asset_x(
        "query:A",
        "query",
        &[],
        json!({ "procedureNames": ["Run"] })
    )]))
    .is_err());
    assert!(normalize_inventory(&inventory(&[asset_x(
        "page:A",
        "page",
        &[],
        json!({ "moduleType": "class" })
    )]))
    .is_err());
    assert!(normalize_inventory(&inventory(&[asset_x(
        "macro:A",
        "macro",
        &[],
        json!({ "procedureCount": 0 })
    )]))
    .is_err());
    assert!(normalize_inventory(&inventory(&[asset_x(
        "external:A",
        "external",
        &[],
        json!({ "hasCodeModule": false })
    )]))
    .is_err());
}

#[test]
fn validates_procedure_name_and_count_limits() {
    let names = |count: usize| (0..count).map(|i| format!("Proc{i}")).collect::<Vec<_>>();
    let names_json =
        |count: usize| Value::Array(names(count).into_iter().map(|n| json!(n)).collect());
    let ok = normalize_inventory(&inventory(&[asset_x(
        "module:大",
        "module",
        &[],
        json!({ "hasCodeModule": true, "moduleType": "standard", "procedureCount": 5000, "procedureNames": names_json(1000) }),
    )]))
    .unwrap();
    assert_eq!(
        ok["assets"][0]["procedureNames"].as_array().unwrap().len(),
        1000
    );
    assert_eq!(ok["assets"][0]["procedureCount"], json!(5000));
    let max_length = normalize_inventory(&inventory(&[asset_x(
        "module:境界",
        "module",
        &[],
        json!({ "hasCodeModule": true, "moduleType": "standard", "procedureCount": 1, "procedureNames": [json!("a".repeat(255))] }),
    )]))
    .unwrap();
    assert_eq!(
        max_length["assets"][0]["procedureNames"][0]
            .as_str()
            .unwrap()
            .len(),
        255
    );
    for extra in [
        json!({ "hasCodeModule": true, "moduleType": "standard", "procedureCount": 5000, "procedureNames": names_json(1001) }),
        json!({ "hasCodeModule": true, "moduleType": "standard", "procedureCount": 100001 }),
        json!({ "hasCodeModule": true, "moduleType": "standard", "procedureNames": [json!("a".repeat(256))] }),
    ] {
        assert!(
            normalize_inventory(&inventory(&[asset_x(
                "module:M",
                "module",
                &[],
                extra.clone()
            )]))
            .is_err(),
            "{extra}"
        );
    }
}

#[test]
fn data_access_pages_unavailable_is_not_treated_as_zero_pages() {
    let input = normalize_inventory(&inventory_x(
        &[asset("form:受注", "form", &[]), asset("report:請求", "report", &[])],
        json!({ "limitations": [{ "code": "DATA_ACCESS_PAGES_UNAVAILABLE", "message": "Access Data Access Pages の一覧を取得できませんでした。不在と取得失敗を区別できないため、ページがないとは判断しません。" }] }),
    ))
    .unwrap();
    assert!(!input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["kind"] == json!("page")));
    assert_eq!(
        input["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["code"] == json!("DATA_ACCESS_PAGES_UNAVAILABLE"))
            .count(),
        1
    );
    assert!(input["limitations"][0]["message"]
        .as_str()
        .unwrap()
        .contains("ページがないとは判断しません"));
    let result = analyze_selection(&input, &sel(&["report:請求"])).unwrap();
    assert_eq!(
        common::strings_of(&result, "selectedIds"),
        vec!["report:請求"]
    );
    assert!(common::has_issue(
        &result,
        "unresolved",
        "DATA_ACCESS_PAGES_UNAVAILABLE"
    ));
    assert!(!common::strings_of(&result, "selectedIds").contains(&"form:受注".to_string()));
}
