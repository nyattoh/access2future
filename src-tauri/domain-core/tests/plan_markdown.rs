mod common;
use common::{asset, asset_x, inventory_x};
use domain_core::{build_plan, merge_inventories, normalize_inventory, render_plan_markdown};
use regex::Regex;
use serde_json::json;

#[test]
fn usage_labels_are_japanese_and_json_identifiers_kept() {
    let demo = common::demo_inventory();
    let plan = build_plan(
        &demo,
        &json!({ "selectedIds": ["form:受注入力"], "usage": common::team_usage(), "targetId": "web" }),
    )
    .unwrap();
    let requirement = plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == json!("利用形態と移行先の制約を確認する"))
        .unwrap();
    assert!(requirement["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| *e == json!("利用人数: 複数人")));
    assert!(requirement["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| *e == json!("同時編集: あり")));
    assert!(requirement["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| *e == json!("利用場所: 社外・遠隔")));
    assert_eq!(plan["usage"]["users"], json!("team"));
    assert_eq!(plan["usage"]["concurrentEditing"], json!(true));
    let text = render_plan_markdown(&plan);
    assert!(text.contains("利用人数: 複数人"));
    assert!(text.contains("オフライン利用: 不要"));
    assert!(text.contains("合成サンプル"));
    assert!(!text.contains("利用人数: team"));
}

#[test]
fn synthetic_sample_plan_has_requirements_steps_and_handover() {
    let input = normalize_inventory(&common::demo_inventory()).unwrap();
    assert_eq!(input["source"]["kind"], json!("synthetic"));
    let form = input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == json!("form"))
        .unwrap();
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": [form["id"]], "usage": common::team_usage(), "targetId": "web", "notes": "段階的に確認する" }),
    )
    .unwrap();
    assert_eq!(plan["version"], json!(1));
    let status = plan["status"].as_str().unwrap();
    assert!(status == "draft" || status == "review-required");
    let requirements = plan["requirements"].as_array().unwrap();
    assert!(!requirements.is_empty());
    for requirement in requirements {
        assert_eq!(requirement["confirmed"], json!(false));
        assert!(!requirement["evidence"].as_array().unwrap().is_empty());
        assert!(requirement["verification"]
            .as_str()
            .is_some_and(|v| !v.is_empty()));
    }
    assert!(!plan["migrationSteps"].as_array().unwrap().is_empty());
    assert!(!plan["validationSteps"].as_array().unwrap().is_empty());
    assert!(plan["risks"].as_array().unwrap().iter().any(|v| {
        let s = v.as_str().unwrap();
        s.contains("未選択") || s.contains("共有")
    }));
    assert!(plan["notices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("合成")));
    assert!(plan["hand_over"].is_object());
    assert_eq!(plan["hand_over"]["targetId"], json!("web"));
    let markdown = render_plan_markdown(&plan);
    assert!(markdown.contains("合成"));
    assert!(markdown.contains("未承認") || markdown.contains("未確認"));
    assert!(markdown.contains("根拠"));
    assert!(markdown.contains("検証"));
}

#[test]
fn unknown_usage_and_analysis_limitations_are_kept_in_plan() {
    let mut input = normalize_inventory(&common::shared_inventory()).unwrap();
    input["limitations"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "code": "PROTECTED", "message": "保護されたオブジェクトが未取得" }));
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "usage": {}, "notes": "<script>alert(1)</script> Password=TOP_SECRET;" }),
    )
    .unwrap();
    assert_eq!(plan["status"], json!("review-required"));
    assert!(common::has_issue(&plan, "unresolved", "PROTECTED"));
    assert!(common::has_issue(&plan, "unresolved", "USAGE_UNKNOWN"));
    let markdown = render_plan_markdown(&plan);
    assert!(!markdown.contains("<script>"));
    assert!(!markdown.contains("TOP_SECRET"));
    assert!(plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["confirmed"] == json!(false)));
}

#[test]
fn undecided_coexistence_with_shared_assets_requires_review() {
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    let mut usage = common::solo_usage();
    usage["coexistence"] = json!("undecided");
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "usage": usage, "targetId": "excel" }),
    )
    .unwrap();
    assert_eq!(plan["status"], json!("review-required"));
    assert!(common::has_issue(
        &plan,
        "unresolved",
        "COEXISTENCE_UNDECIDED"
    ));
    assert!(!plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["confirmed"] == json!(true)));
}

#[test]
fn plan_contains_mermaid_flow_and_er_diagrams() {
    let input = inventory_x(
        &[
            asset("form:受注", "form", &["table:受注"]),
            asset("form:顧客", "form", &["table:受注"]),
            asset_x(
                "table:受注",
                "table",
                &[],
                json!({ "fields": [
                    { "name": "受注番号", "dataType": "Long", "isPrimaryKey": true },
                    { "name": "顧客\"ID", "dataType": "長整数" }
                ] }),
            ),
            asset_x(
                "table:顧客",
                "table",
                &[],
                json!({ "fields": [{ "name": "顧客ID", "dataType": "Long", "isPrimaryKey": true }] }),
            ),
            asset("table:無関係", "table", &[]),
        ],
        json!({ "relations": [{ "from": "table:受注", "to": "table:顧客", "fields": [{ "from": "顧客ID", "to": "顧客ID" }] }] }),
    );
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:受注"], "usage": common::solo_usage() }),
    )
    .unwrap();
    let flow = plan["diagrams"]["flow"].as_str().unwrap();
    let er = plan["diagrams"]["er"].as_str().unwrap();
    assert!(flow.starts_with("flowchart LR"));
    assert!(flow.contains("form: 受注"));
    assert!(
        flow.contains("form: 顧客"),
        "共有資産に依存する未選択画面も点線で示す"
    );
    assert!(er.starts_with("erDiagram"));
    assert!(er.contains("||--o{"));
    assert!(er.contains("顧客#quot;ID"));
    assert!(!er.contains("無関係"));
    let text = render_plan_markdown(&plan);
    assert!(text.contains("```mermaid\nflowchart LR"));
    assert!(text.contains("```mermaid\nerDiagram"));
}

#[test]
fn er_diagram_is_empty_when_dependency_scope_has_no_tables() {
    let input = inventory_x(&[asset("form:A", "form", &[])], json!({}));
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:A"], "usage": common::solo_usage() }),
    )
    .unwrap();
    assert_eq!(plan["diagrams"]["er"], json!(""));
    let markdown = render_plan_markdown(&plan);
    assert!(markdown.contains("### ER図（依存範囲のテーブル）\n\n依存範囲にテーブルはありません。"));
    assert!(!markdown.contains("```mermaid\nerDiagram"));
}

#[test]
fn impacted_unselected_screens_show_paths_via_intermediate_assets() {
    let demo = common::demo_inventory();
    let plan = build_plan(
        &demo,
        &json!({ "selectedIds": ["form:受注入力"], "usage": common::solo_usage() }),
    )
    .unwrap();
    let flow = plan["diagrams"]["flow"].as_str().unwrap();
    let id_of = |name: &str| -> String {
        let re = Regex::new(&format!(r#"(n\d+)\[\(?"[a-z]+: {name}""#)).unwrap();
        re.captures(flow)
            .unwrap_or_else(|| panic!("図に {name} があること"))[1]
            .to_string()
    };
    let lines: Vec<String> = flow.split('\n').map(|l| l.trim().to_string()).collect();
    assert!(lines.contains(&format!("{} -.-> {}", id_of("請求書"), id_of("請求対象"))));
    assert!(lines.contains(&format!(
        "{} -.->|影響候補| {}",
        id_of("請求対象"),
        id_of("受注")
    )));
}

#[test]
fn requirements_are_numbered_and_tables_listed_with_field_counts() {
    let input = merge_inventories(&common::split_database()).unwrap();
    let form = input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == json!("form"))
        .unwrap();
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": [form["id"]], "usage": common::solo_usage() }),
    )
    .unwrap();
    let ids: Vec<&str> = plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.first(), Some(&"REQ-001"));
    assert_eq!(ids.len(), plan["requirements"].as_array().unwrap().len());
    let data_req = plan["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == json!("データ構造と参照整合性を維持する"))
        .expect("テーブル要件があること");
    let evidence: Vec<&str> = data_req["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect();
    assert!(evidence.iter().any(|e| e.contains("2項目")));
}

#[test]
fn markdown_of_minimal_review_plan_lists_no_unresolved_fallback() {
    let input = normalize_inventory(&common::inventory(&[asset("form:A", "form", &[])])).unwrap();
    let plan = build_plan(
        &input,
        &json!({ "selectedIds": ["form:A"], "usage": common::solo_usage(), "targetId": "excel" }),
    )
    .unwrap();
    let markdown = render_plan_markdown(&plan);
    assert!(markdown.contains("# 移行計画（草案）"));
    assert!(markdown.contains("状態: 草案 / 要件は未承認"));
    assert!(markdown.contains("## 引き継ぎ"));
    assert!(markdown.ends_with('\n'));
    assert!(markdown.contains("- 取得済み情報からの指摘なし。"));
    assert!(!markdown.contains("handover"));
}
