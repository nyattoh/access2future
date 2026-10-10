mod common;
use domain_core::{analyze_selection, normalize_inventory, recommend_targets};
use serde_json::{json, Value};

#[test]
fn candidates_and_constraints_per_usage_with_reasons() {
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    let analysis = analyze_selection(&input, &common::sel(&["form:受注"])).unwrap();
    let by_id = |usage: &Value| -> std::collections::HashMap<String, Value> {
        recommend_targets(&input, usage, &analysis)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["id"].as_str().unwrap().to_string(), c.clone()))
            .collect()
    };
    let solo = by_id(&common::solo_usage());
    assert_eq!(solo["excel"]["fit"], json!("recommended"));
    assert_eq!(solo["sheets-gas"]["fit"], json!("not-recommended"));
    let team = by_id(&common::team_usage());
    assert_eq!(team["web"]["fit"], json!("recommended"));
    assert_eq!(team["excel"]["fit"], json!("not-recommended"));
    let mut same = common::team_usage();
    same["permissions"] = json!("same");
    assert_eq!(by_id(&same)["sheets-gas"]["fit"], json!("recommended"));
    let mut offline = common::team_usage();
    offline["offlineRequired"] = json!(true);
    assert_eq!(by_id(&offline)["web"]["fit"], json!("conditional"));
    let mut lan = common::solo_usage();
    lan["users"] = json!("team");
    lan["concurrentEditing"] = json!(true);
    lan["location"] = json!("lan");
    lan["offlineRequired"] = json!(false);
    assert_eq!(by_id(&lan)["web"]["fit"], json!("recommended"));
    let mut shared_store = common::team_usage();
    shared_store["coexistence"] = json!("shared-store");
    assert!(by_id(&shared_store)["web"]["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| {
            let s = t.as_str().unwrap();
            s.contains("共有") || s.contains("同期")
        }));
    for item in recommend_targets(&input, &json!({}), &analysis)
        .unwrap()
        .as_array()
        .unwrap()
    {
        assert_eq!(item["fit"], json!("conditional"));
        assert!(!item["unknowns"].as_array().unwrap().is_empty());
        assert!(!item["reasons"].as_array().unwrap().is_empty());
    }
}

#[test]
fn candidate_shape_is_stable() {
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    let analysis = analyze_selection(&input, &common::sel(&["form:受注"])).unwrap();
    let candidates = recommend_targets(&input, &common::team_usage(), &analysis).unwrap();
    let list = candidates.as_array().unwrap();
    assert_eq!(list.len(), 3);
    let ids: Vec<&str> = list.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["web", "excel", "sheets-gas"]);
    for c in list {
        for key in ["id", "label", "fit", "reasons", "requirements", "unknowns"] {
            assert!(c.get(key).is_some(), "{key}");
        }
        assert_eq!(c["label"].as_str().unwrap().is_empty(), false);
    }
}

#[test]
fn impacted_analysis_adds_shared_asset_constraint() {
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    let analysis = analyze_selection(&input, &common::sel(&["form:受注"])).unwrap();
    let candidates = recommend_targets(&input, &common::team_usage(), &analysis).unwrap();
    for c in candidates.as_array().unwrap() {
        assert!(c["requirements"].as_array().unwrap().iter().any(|r| r
            .as_str()
            .unwrap()
            .contains("共有資産に依存する未選択の機能")));
    }
    let no_impact = json!({ "impactedIds": [], "sharedIds": [] });
    let candidates = recommend_targets(&input, &common::team_usage(), &no_impact).unwrap();
    for c in candidates.as_array().unwrap() {
        assert!(!c["requirements"].as_array().unwrap().iter().any(|r| r
            .as_str()
            .unwrap()
            .contains("共有資産に依存する未選択の機能")));
    }
}

#[test]
fn invalid_inventory_or_usage_is_rejected() {
    let analysis = json!({ "impactedIds": [] });
    assert!(recommend_targets(
        &json!({ "schemaVersion": 2 }),
        &common::solo_usage(),
        &analysis
    )
    .is_err());
    let input = normalize_inventory(&common::shared_inventory()).unwrap();
    assert!(recommend_targets(&input, &json!(null), &analysis).is_err());
    assert!(recommend_targets(&input, &json!({ "users": "unknown" }), &analysis).is_err());
}
