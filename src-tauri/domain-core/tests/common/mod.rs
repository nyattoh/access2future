#![allow(dead_code)]
use serde_json::{json, Map, Value};

pub fn asset(id: &str, kind: &str, depends_on: &[&str]) -> Value {
    asset_x(id, kind, depends_on, json!({}))
}

pub fn asset_x(id: &str, kind: &str, depends_on: &[&str], extra: Value) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("kind".into(), json!(kind));
    m.insert("name".into(), json!(id.splitn(2, ':').nth(1).unwrap_or("")));
    m.insert("dependsOn".into(), json!(depends_on));
    m.insert("status".into(), json!("supported"));
    m.insert("issues".into(), json!([]));
    merge_extra(&mut m, extra);
    Value::Object(m)
}

pub fn inventory(assets: &[Value]) -> Value {
    inventory_x(assets, json!({}))
}

pub fn inventory_x(assets: &[Value], extra: Value) -> Value {
    let mut m = Map::new();
    m.insert("schemaVersion".into(), json!(1));
    m.insert(
        "source".into(),
        json!({ "name": "業務.accdb", "kind": "inventory" }),
    );
    m.insert("assets".into(), Value::Array(assets.to_vec()));
    m.insert("relations".into(), json!([]));
    m.insert("limitations".into(), json!([]));
    merge_extra(&mut m, extra);
    Value::Object(m)
}

fn merge_extra(m: &mut Map<String, Value>, extra: Value) {
    if let Value::Object(e) = extra {
        for (k, v) in e {
            m.insert(k, v);
        }
    }
}

pub fn solo_usage() -> Value {
    json!({
        "users": "solo", "concurrentEditing": false, "location": "device",
        "offlineRequired": true, "permissions": "same", "coexistence": "replace-scope"
    })
}

pub fn team_usage() -> Value {
    json!({
        "users": "team", "concurrentEditing": true, "location": "remote",
        "offlineRequired": false, "permissions": "roles", "coexistence": "keep-source"
    })
}

pub fn shared_inventory() -> Value {
    inventory_x(
        &[
            asset("form:受注", "form", &["query:受注"]),
            asset("query:受注", "query", &["table:受注"]),
            asset("table:受注", "table", &[]),
            asset("table:顧客", "table", &[]),
            asset("form:顧客", "form", &["table:顧客"]),
            asset("report:請求", "report", &["table:受注"]),
            asset("page:在庫", "page", &["table:在庫"]),
            asset("table:在庫", "table", &[]),
        ],
        json!({ "relations": [{ "from": "table:受注", "to": "table:顧客", "fields": [{ "from": "顧客ID", "to": "ID" }], "enforced": true }] }),
    )
}

pub fn split_database() -> Vec<Value> {
    let frontend = inventory_x(
        &[
            asset("form:受注入力", "form", &["table:受注リンク"]),
            asset("report:請求書", "report", &["table:受注リンク"]),
            asset_x(
                "table:受注リンク",
                "table",
                &[],
                json!({ "isLinked": true, "linkedDatabaseName": "backend.accdb", "linkedTableName": "受注" }),
            ),
        ],
        json!({ "source": { "name": "frontend.mdb", "kind": "synthetic" } }),
    );
    let backend = inventory_x(
        &[
            asset_x(
                "table:受注",
                "table",
                &[],
                json!({ "local": true, "fields": [
                    { "name": "受注ID", "dataType": "Long", "required": true, "isPrimaryKey": true },
                    { "name": "顧客ID", "dataType": "Long", "required": true, "isPrimaryKey": false }
                ] }),
            ),
            asset_x(
                "table:顧客",
                "table",
                &[],
                json!({ "local": true, "fields": [
                    { "name": "顧客ID", "dataType": "Long", "required": true, "isPrimaryKey": true }
                ] }),
            ),
            asset("form:顧客管理", "form", &["table:顧客"]),
        ],
        json!({
            "source": { "name": "backend.accdb", "kind": "synthetic" },
            "relations": [{ "from": "table:受注", "to": "table:顧客", "fields": [{ "from": "顧客ID", "to": "顧客ID" }], "enforced": true }]
        }),
    );
    vec![frontend, backend]
}

pub fn demo_inventory() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo.inventory.json");
    let text = std::fs::read_to_string(&path).expect("demo.inventory.json を読み込めること");
    serde_json::from_str(&text).expect("demo.inventory.json がJSONとして妥当であること")
}

pub fn sel(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

pub fn strings_of(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} は配列であること"))
        .iter()
        .map(|x| {
            x.as_str()
                .unwrap_or_else(|| panic!("{key} の要素は文字列であること"))
                .to_string()
        })
        .collect()
}

pub fn has_issue(v: &Value, list: &str, code: &str) -> bool {
    v[list]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|i| i["code"] == json!(code))
}

pub fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}
