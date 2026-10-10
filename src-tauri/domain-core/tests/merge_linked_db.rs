mod common;
use common::{asset_x, inventory_x};
use domain_core::{analyze_selection, build_plan, merge_inventories, normalize_inventory};
use serde_json::{json, Value};

fn find_by_name<'a>(merged: &'a Value, name: &str) -> &'a Value {
    merged["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == json!(name))
        .unwrap_or_else(|| panic!("資産 {name} が見つかること"))
}

#[test]
fn split_database_alias_linked_only_by_explicit_link_with_pk_fk_and_impacts() {
    let materials = common::split_database();
    let originals = serde_json::to_string(&materials).unwrap();
    let merged = merge_inventories(&materials).unwrap();
    assert_eq!(merged["source"]["kind"], json!("inventory"));
    assert_eq!(
        merged["source"]["name"],
        json!("frontend.mdb + backend.accdb")
    );
    let form = find_by_name(&merged, "受注入力").clone();
    let alias = find_by_name(&merged, "受注リンク").clone();
    let orders = find_by_name(&merged, "受注").clone();
    let customer = find_by_name(&merged, "顧客").clone();
    let customer_form = find_by_name(&merged, "顧客管理").clone();
    assert_eq!(alias["linkedDatabaseName"], json!("backend.accdb"));
    assert_eq!(alias["linkedTableName"], json!("受注"));
    assert!(alias["fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == json!("受注ID") && f["isPrimaryKey"] == json!(true)));
    assert!(alias["dependsOn"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| *d == orders["id"]));
    assert!(merged["relations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["from"] == orders["id"]
            && r["to"] == customer["id"]
            && r["fields"][0]["from"] == json!("顧客ID")));
    let analysis = analyze_selection(&merged, &[form["id"].as_str().unwrap().to_string()]).unwrap();
    let deps = common::strings_of(&analysis, "dependencyIds");
    assert!(deps.contains(&orders["id"].as_str().unwrap().to_string()));
    assert!(deps.contains(&customer["id"].as_str().unwrap().to_string()));
    let impacted = common::strings_of(&analysis, "impactedIds");
    assert!(impacted.contains(&customer_form["id"].as_str().unwrap().to_string()));
    let report = merged["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == json!("report"))
        .unwrap();
    assert!(impacted.contains(&report["id"].as_str().unwrap().to_string()));
    assert_eq!(analysis["selectedIds"], json!([form["id"]]));
    assert!(customer_form["caption"]
        .as_str()
        .unwrap()
        .contains("backend.accdb"));
    assert_eq!(serde_json::to_string(&materials).unwrap(), originals);
    let plan = build_plan(
        &merged,
        &json!({ "selectedIds": [form["id"]], "usage": common::solo_usage() }),
    )
    .unwrap();
    assert!(plan["notices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("確認していません")));
}

#[test]
fn reversed_input_order_still_links_and_first_material_becomes_primary() {
    let [frontend, backend] = common::split_database().try_into().unwrap();
    let merged = merge_inventories(&[backend, frontend]).unwrap();
    assert_eq!(
        merged["source"]["name"],
        json!("backend.accdb + frontend.mdb")
    );
    assert!(merged["assets"][0]["caption"]
        .as_str()
        .unwrap()
        .contains("backend.accdb"));
    let form = find_by_name(&merged, "受注入力");
    let analysis = analyze_selection(&merged, &[form["id"].as_str().unwrap().to_string()]).unwrap();
    let customer = find_by_name(&merged, "顧客");
    assert!(common::strings_of(&analysis, "dependencyIds")
        .contains(&customer["id"].as_str().unwrap().to_string()));
}

#[test]
fn same_name_tables_in_different_dbs_are_not_conflated() {
    let [frontend, backend] = common::split_database().try_into().unwrap();
    let other = inventory_x(
        &[asset_x(
            "table:受注",
            "table",
            &[],
            json!({ "local": true, "fields": [{ "name": "別DB専用", "dataType": "Text" }] }),
        )],
        json!({ "source": { "name": "other.accdb", "kind": "synthetic" } }),
    );
    let merged = merge_inventories(&[frontend, backend, other]).unwrap();
    let orders: Vec<&Value> = merged["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["name"] == json!("受注"))
        .collect();
    assert_eq!(orders.len(), 2);
    assert_ne!(orders[0]["id"], orders[1]["id"]);
    let alias = find_by_name(&merged, "受注リンク");
    assert!(alias["fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == json!("受注ID")));
    assert!(!alias["fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == json!("別DB専用")));
    assert!(common::has_issue(
        &merged,
        "limitations",
        "TABLE_NAME_COLLISION"
    ));
}

#[test]
fn missing_backend_duplicate_db_and_duplicate_table_stay_unresolved() {
    let [frontend, backend] = common::split_database().try_into().unwrap();
    let mut scenarios: Vec<(Vec<Value>, &str)> = vec![
        (vec![frontend.clone()], "LINK_BACKEND_MISSING"),
        (
            vec![frontend.clone(), backend.clone(), backend.clone()],
            "LINK_AMBIGUOUS",
        ),
        (
            vec![
                frontend.clone(),
                inventory_x(&[], json!({ "source": backend["source"].clone() })),
            ],
            "LINK_TABLE_MISSING",
        ),
        (
            vec![
                frontend.clone(),
                inventory_x(
                    &[
                        backend["assets"].as_array().unwrap().clone(),
                        vec![asset_x(
                            "table:受注別ID",
                            "table",
                            &[],
                            json!({ "name": "受注", "local": true }),
                        )],
                    ]
                    .concat(),
                    json!({ "source": backend["source"].clone() }),
                ),
            ],
            "LINK_AMBIGUOUS",
        ),
    ];
    let mut missing = frontend.clone();
    missing["assets"][2]
        .as_object_mut()
        .unwrap()
        .remove("linkedTableName");
    scenarios.push((vec![missing, backend.clone()], "LINK_METADATA_MISSING"));
    for (materials, code) in scenarios {
        let merged = merge_inventories(&materials).unwrap();
        let alias = find_by_name(&merged, "受注リンク");
        assert!(alias["dependsOn"].as_array().unwrap().is_empty(), "{code}");
        assert!(
            merged["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["code"] == json!(code) && l["assetId"] == alias["id"]),
            "{code}"
        );
        let form = find_by_name(&merged, "受注入力");
        assert!(
            common::has_issue(
                &analyze_selection(&merged, &[form["id"].as_str().unwrap().to_string()]).unwrap(),
                "unresolved",
                code
            ),
            "{code}"
        );
    }
}

#[test]
fn safe_link_metadata_kept_and_paths_types_empty_values_rejected() {
    let good = normalize_inventory(&common::split_database()[0]).unwrap();
    assert_eq!(
        good["assets"][2]["linkedDatabaseName"],
        json!("backend.accdb")
    );
    assert_eq!(good["assets"][2]["linkedTableName"], json!("受注"));
    for (key, value) in [
        ("linkedDatabaseName", json!("C:\\private\\backend.accdb")),
        ("linkedDatabaseName", json!("/private/backend.accdb")),
        ("linkedDatabaseName", json!("../backend.accdb")),
        ("linkedDatabaseName", json!("")),
        ("linkedDatabaseName", json!(2)),
        ("linkedTableName", json!("")),
        ("linkedTableName", json!(false)),
    ] {
        let mut input = common::split_database()[0].clone();
        input["assets"][2][key] = value.clone();
        assert!(
            normalize_inventory(&input).is_err(),
            "{key} = {value} は拒否すること"
        );
    }
    assert!(merge_inventories(&[]).is_err());
}

#[test]
fn single_inventory_merge_still_normalizes_and_documents_metadata_only() {
    let materials = common::split_database();
    let merged = merge_inventories(&[materials[0].clone()]).unwrap();
    assert!(common::has_issue(
        &merged,
        "limitations",
        "MERGED_METADATA_ONLY"
    ));
    assert!(common::has_issue(
        &merged,
        "limitations",
        "LINK_BACKEND_MISSING"
    ));
    assert_eq!(merged["schemaVersion"], json!(1));
}
