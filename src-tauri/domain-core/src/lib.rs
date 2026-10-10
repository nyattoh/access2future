use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

const ASSET_KINDS: [&str; 8] = [
    "table", "query", "form", "page", "report", "macro", "module", "external",
];
const STATUSES: [&str; 3] = ["supported", "partial", "unsupported"];
const SOURCE_KINDS: [&str; 3] = ["access", "inventory", "synthetic"];
const CODE_KINDS: [&str; 3] = ["form", "report", "module"];
const SELECTION_KINDS: [&str; 3] = ["form", "page", "report"];
const USAGE_KEYS: [&str; 6] = [
    "users",
    "concurrentEditing",
    "location",
    "offlineRequired",
    "permissions",
    "coexistence",
];

fn usage_label(key: &str) -> &'static str {
    match key {
        "users" => "利用人数",
        "concurrentEditing" => "同時編集",
        "location" => "利用場所",
        "offlineRequired" => "オフライン利用",
        "permissions" => "権限",
        _ => "既存Accessとの共存",
    }
}

fn usage_choices(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "users" => Some(&["solo", "team"]),
        "location" => Some(&["device", "lan", "remote"]),
        "permissions" => Some(&["same", "roles"]),
        "coexistence" => Some(&["undecided", "keep-source", "shared-store", "replace-scope"]),
        _ => None,
    }
}

static RE_CONTROL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]").unwrap());
static RE_CONNECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?-u:\b)(?:Provider|Data Source|Server|DSN|Database|DBQ|UID|User ID|UserName|PWD|Password|Jet OLEDB:Database Password|token|api[_-]?key|secret)\s*=\s*(?:"(?:[^"]|"")*"|'(?:[^']|'')*'|[^;\r\n])*"#,
    )
    .unwrap()
});
static RE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?-u:\b)(?:postgres(?:ql)?|mysql|sqlserver|mongodb(?:\+srv)?|https?)://[^\s<>]+",
    )
    .unwrap()
});
static RE_WIN_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:[A-Za-z]:[\\/]|\\\\)[^\s<>"']+"#).unwrap());
static RE_UNIX_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|\s)/(?:[^\s<>"']+/)*[^\s<>"']+"#).unwrap());
static RE_PROCEDURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}_][\p{L}\p{N}_]*$").unwrap());
static RE_FINGERPRINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-f0-9]{64}$").unwrap());
static RE_ISO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})?$").unwrap()
});
static RE_ER_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap());

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn as_object<'a>(v: Option<&'a Value>, label: &str) -> Result<&'a Map<String, Value>, String> {
    v.and_then(Value::as_object)
        .ok_or_else(|| format!("{label}はオブジェクトで指定してください。"))
}

fn as_array<'a>(v: Option<&'a Value>, label: &str, max: usize) -> Result<&'a [Value], String> {
    let err = || format!("{label}は{max}件以下の配列を指定してください。");
    let arr = v.and_then(Value::as_array).ok_or_else(err)?;
    if arr.len() > max {
        return Err(err());
    }
    Ok(arr)
}

fn req_string<'a>(v: Option<&'a Value>, label: &str, max: usize) -> Result<&'a str, String> {
    let err = || format!("{label}は空でない{max}文字以下の文字列を指定してください。");
    let s = v.and_then(Value::as_str).ok_or_else(err)?;
    if utf16_len(s) > max || s.trim().is_empty() {
        return Err(err());
    }
    Ok(s)
}

fn redact(s: &str) -> String {
    let t = RE_CONTROL.replace_all(s, "");
    let t = RE_CONNECTION.replace_all(&t, "[接続情報を除去]");
    let t = RE_URL.replace_all(&t, "[外部参照を除去]");
    let t = RE_WIN_PATH.replace_all(&t, "[元のパスを除去]");
    RE_UNIX_PATH
        .replace_all(&t, " [元のパスを除去]")
        .into_owned()
}

fn safe_text_of(s: &str, label: &str, max: usize) -> Result<String, String> {
    if utf16_len(s) > max || s.trim().is_empty() {
        return Err(format!(
            "{label}は空でない{max}文字以下の文字列を指定してください。"
        ));
    }
    Ok(redact(s))
}

fn safe_text(v: Option<&Value>, label: &str, max: usize) -> Result<String, String> {
    safe_text_of(req_string(v, label, max)?, label, max)
}

fn safe_id_str(id: &str, label: &str) -> Result<String, String> {
    if utf16_len(id) > 1000 || id.trim().is_empty() {
        return Err(format!(
            "{label}は空でない1000文字以下の文字列を指定してください。"
        ));
    }
    let has_control = id.chars().any(|c| {
        let n = c as u32;
        n <= 0x1f || n == 0x7f
    });
    if has_control || redact(id) != id {
        return Err(format!("{label}に秘密情報や絶対パスを含めないでください。"));
    }
    Ok(id.to_string())
}

fn safe_id(v: Option<&Value>, label: &str) -> Result<String, String> {
    safe_id_str(req_string(v, label, 1000)?, label)
}

fn linked_name(v: Option<&Value>, label: &str, filename: bool) -> Result<String, String> {
    let name = safe_id(v, label)?;
    let unsafe_chars = filename && name.chars().any(|c| "<>:\"|?*".contains(c));
    if utf16_len(&name) > 255
        || name.contains('/')
        || name.contains('\\')
        || unsafe_chars
        || (filename && (name == "." || name == ".."))
    {
        return Err(format!(
            "{label}はパスを含まない安全な名前で指定してください。"
        ));
    }
    Ok(name)
}

fn procedure_name(v: &Value, label: &str) -> Result<String, String> {
    let err = || {
        format!("{label}はプロシージャ名（255字以下の安全な識別子）だけを指定してください。ソース本文・パス・秘密は含めないでください。")
    };
    let Some(s) = v.as_str() else {
        return Err(err());
    };
    if utf16_len(s) > 255 || !RE_PROCEDURE.is_match(s) {
        return Err(err());
    }
    Ok(s.to_string())
}

fn choice<'a>(v: Option<&'a Value>, choices: &[&str], label: &str) -> Result<&'a str, String> {
    let err = || format!("{label}の値が不正です。");
    let s = v.and_then(Value::as_str).ok_or_else(err)?;
    if choices.contains(&s) {
        Ok(s)
    } else {
        Err(err())
    }
}

fn req_bool(v: Option<&Value>, label: &str) -> Result<bool, String> {
    v.and_then(Value::as_bool)
        .ok_or_else(|| format!("{label}は真偽値を指定してください。"))
}

fn safe_integer(v: &Value) -> Option<i64> {
    let f = v.as_f64()?;
    if f.fract() != 0.0 || f.abs() > 9_007_199_254_740_991.0 {
        return None;
    }
    Some(f as i64)
}

fn issue_value(v: Option<&Value>, label: &str) -> Result<Value, String> {
    let o = as_object(v, label)?;
    let mut m = Map::new();
    m.insert(
        "code".into(),
        json!(safe_id(o.get("code"), &format!("{label}.code"))?),
    );
    m.insert(
        "message".into(),
        json!(safe_text(
            o.get("message"),
            &format!("{label}.message"),
            4000
        )?),
    );
    if let Some(a) = o.get("assetId") {
        m.insert(
            "assetId".into(),
            json!(safe_id(Some(a), &format!("{label}.assetId"))?),
        );
    }
    Ok(Value::Object(m))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_iso_datetime(s: &str) -> bool {
    if !RE_ISO.is_match(s) {
        return false;
    }
    let year: i32 = s[0..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

pub fn normalize_inventory(input: &Value) -> Result<Value, String> {
    let obj = as_object(Some(input), "Inventory")?;
    let ok_version = obj
        .get("schemaVersion")
        .is_some_and(|v| v.as_i64() == Some(1) || v.as_f64() == Some(1.0));
    if !ok_version {
        return Err("InventoryのschemaVersionは1にしてください。".into());
    }
    let source_obj = as_object(obj.get("source"), "source")?;
    let raw_name = req_string(source_obj.get("name"), "source.name", 1000)?;
    let base_name = raw_name
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or("");
    let mut source = Map::new();
    source.insert(
        "name".into(),
        json!(safe_text_of(base_name, "source.name", 4000)?),
    );
    source.insert(
        "kind".into(),
        json!(choice(
            source_obj.get("kind"),
            &SOURCE_KINDS,
            "source.kind"
        )?),
    );
    if let Some(v) = source_obj.get("accessVersion") {
        source.insert(
            "accessVersion".into(),
            json!(safe_text(Some(v), "source.accessVersion", 100)?),
        );
    }
    if let Some(v) = source_obj.get("analysedAt") {
        let date = req_string(Some(v), "source.analysedAt", 100)?;
        if !is_iso_datetime(date) {
            return Err("source.analysedAtはISO日時で指定してください。".into());
        }
        source.insert("analysedAt".into(), json!(date));
    }
    if let Some(v) = source_obj.get("fingerprint") {
        let fp = v
            .as_str()
            .filter(|s| RE_FINGERPRINT.is_match(s))
            .ok_or("source.fingerprintはSHA256で指定してください。")?;
        source.insert("fingerprint".into(), json!(fp));
    }

    let mut ids: HashSet<String> = HashSet::new();
    let mut edge_count = 0usize;
    let mut assets_out: Vec<Value> = Vec::new();
    for (index, value) in as_array(obj.get("assets"), "assets", 5000)?
        .iter()
        .enumerate()
    {
        let label = format!("assets[{index}]");
        let a = as_object(Some(value), &label)?;
        let id = safe_id(a.get("id"), &format!("{label}.id"))?;
        if !ids.insert(id.clone()) {
            return Err(format!("重複した資産IDがあります: {id}"));
        }
        let kind = choice(a.get("kind"), &ASSET_KINDS, &format!("{label}.kind"))?;
        let mut m = Map::new();
        m.insert("id".into(), json!(id));
        m.insert("kind".into(), json!(kind));
        m.insert(
            "name".into(),
            json!(safe_text(a.get("name"), &format!("{label}.name"), 500)?),
        );
        let dep_values: &[Value] = match a.get("dependsOn") {
            None => &[],
            Some(v) => as_array(Some(v), &format!("{label}.dependsOn"), 5000)?,
        };
        let mut deps: Vec<String> = Vec::new();
        let mut dep_seen: HashSet<String> = HashSet::new();
        for d in dep_values {
            let did = safe_id(Some(d), &format!("{label}.dependsOn"))?;
            if dep_seen.insert(did.clone()) {
                deps.push(did);
            }
        }
        edge_count += deps.len();
        if edge_count > 50000 {
            return Err("依存参照は合計50000件以下にしてください。".into());
        }
        m.insert("dependsOn".into(), json!(deps));
        let status = match a.get("status") {
            None => "partial",
            Some(v) => choice(Some(v), &STATUSES, &format!("{label}.status"))?,
        };
        m.insert("status".into(), json!(status));
        let issue_values: &[Value] = match a.get("issues") {
            None => &[],
            Some(v) => as_array(Some(v), &format!("{label}.issues"), 500)?,
        };
        let mut issues = Vec::new();
        for item in issue_values {
            issues.push(issue_value(Some(item), &format!("{label}.issues"))?);
        }
        m.insert("issues".into(), Value::Array(issues));
        if let Some(v) = a.get("caption") {
            m.insert(
                "caption".into(),
                json!(safe_text(Some(v), &format!("{label}.caption"), 500)?),
            );
        }
        for key in ["isLinked", "linked", "local"] {
            if let Some(v) = a.get(key) {
                m.insert(
                    key.into(),
                    json!(req_bool(Some(v), &format!("{label}.{key}"))?),
                );
            }
        }
        for key in ["linkedDatabaseName", "linkedTableName"] {
            if let Some(v) = a.get(key) {
                if kind != "table" {
                    return Err(format!("{key}はtableだけに指定できます。"));
                }
                let name = linked_name(
                    Some(v),
                    &format!("{label}.{key}"),
                    key == "linkedDatabaseName",
                )?;
                m.insert(key.into(), json!(name));
                m.insert("isLinked".into(), json!(true));
            }
        }
        if let Some(v) = a.get("fields") {
            if kind != "table" {
                return Err("fieldsはtableだけに指定できます。".into());
            }
            let mut fields = Vec::new();
            for field in as_array(Some(v), &format!("{label}.fields"), 1000)? {
                let fo = as_object(Some(field), &format!("{label}.fields"))?;
                let mut fm = Map::new();
                fm.insert(
                    "name".into(),
                    json!(safe_text(fo.get("name"), "field.name", 500)?),
                );
                if let Some(dt) = fo.get("dataType") {
                    fm.insert(
                        "dataType".into(),
                        json!(safe_text(Some(dt), "field.dataType", 100)?),
                    );
                }
                for key in ["required", "isPrimaryKey"] {
                    if let Some(b) = fo.get(key) {
                        fm.insert(
                            key.into(),
                            json!(req_bool(Some(b), &format!("field.{key}"))?),
                        );
                    }
                }
                fields.push(Value::Object(fm));
            }
            m.insert("fields".into(), Value::Array(fields));
        }
        if let Some(v) = a.get("rowCount") {
            let n = safe_integer(v)
                .filter(|n| *n >= 0)
                .ok_or("rowCountはtableの0以上の整数で指定してください。")?;
            if kind != "table" {
                return Err("rowCountはtableの0以上の整数で指定してください。".into());
            }
            let external = m.get("isLinked").and_then(Value::as_bool) == Some(true)
                || m.get("linked").and_then(Value::as_bool) == Some(true)
                || m.get("local").and_then(Value::as_bool) == Some(false);
            if !external {
                m.insert("rowCount".into(), json!(n));
            }
        }
        let code_keys = [
            "hasCodeModule",
            "moduleType",
            "procedureNames",
            "procedureCount",
        ];
        if code_keys.iter().any(|k| a.contains_key(*k)) {
            if !CODE_KINDS.contains(&kind) {
                return Err("VBA要約はフォーム・帳票・モジュールだけに指定できます。ページはAccess Data Access Pagesのため非対応です。".into());
            }
            if let Some(v) = a.get("hasCodeModule") {
                m.insert(
                    "hasCodeModule".into(),
                    json!(req_bool(Some(v), &format!("{label}.hasCodeModule"))?),
                );
            }
            if let Some(v) = a.get("moduleType") {
                let choices: &[&str] = if kind == "module" {
                    &["standard", "class"]
                } else {
                    &["class"]
                };
                m.insert(
                    "moduleType".into(),
                    json!(choice(Some(v), choices, &format!("{label}.moduleType"))?),
                );
            }
            if let Some(v) = a.get("procedureNames") {
                let names = as_array(Some(v), &format!("{label}.procedureNames"), 1000)?;
                let mut seen: HashSet<&Value> = HashSet::new();
                if names.iter().any(|n| !seen.insert(n)) {
                    return Err(format!(
                        "{label}.procedureNamesは重複しない識別子で指定してください。"
                    ));
                }
                let mut arr = Vec::new();
                for n in names {
                    arr.push(json!(procedure_name(
                        n,
                        &format!("{label}.procedureNames")
                    )?));
                }
                m.insert("procedureNames".into(), Value::Array(arr));
            }
            if let Some(v) = a.get("procedureCount") {
                let n = safe_integer(v)
                    .filter(|n| (0..=100000).contains(n))
                    .ok_or_else(|| {
                        format!("{label}.procedureCountは0以上100000以下の整数を指定してください。")
                    })?;
                m.insert("procedureCount".into(), json!(n));
            }
            if m.get("moduleType").is_some() && !a.contains_key("hasCodeModule") {
                m.insert("hasCodeModule".into(), json!(true));
            }
            let has_procedures = m
                .get("procedureNames")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
                || m.get("procedureCount")
                    .and_then(Value::as_i64)
                    .is_some_and(|c| c > 0);
            if m.get("hasCodeModule").and_then(Value::as_bool) == Some(false)
                && (m.get("moduleType").is_some() || has_procedures)
            {
                return Err("hasCodeModuleがfalseの資産にコードの要約を指定できません。".into());
            }
            if let (Some(count), Some(names)) = (
                m.get("procedureCount").and_then(Value::as_i64),
                m.get("procedureNames").and_then(Value::as_array),
            ) {
                if (count as usize) < names.len() {
                    return Err(format!(
                        "{label}.procedureCountはprocedureNamesの件数以上にしてください。"
                    ));
                }
            }
        }
        assets_out.push(Value::Object(m));
    }

    let relation_values: &[Value] = match obj.get("relations") {
        None => &[],
        Some(v) => as_array(Some(v), "relations", 10000)?,
    };
    let mut relations_out = Vec::new();
    for r in relation_values {
        let ro = as_object(Some(r), "relation")?;
        let mut rm = Map::new();
        rm.insert(
            "from".into(),
            json!(safe_id(ro.get("from"), "relation.from")?),
        );
        rm.insert("to".into(), json!(safe_id(ro.get("to"), "relation.to")?));
        let field_values: &[Value] = match ro.get("fields") {
            None => &[],
            Some(v) => as_array(Some(v), "relation.fields", 1000)?,
        };
        let mut fields = Vec::new();
        for f in field_values {
            let fo = as_object(Some(f), "relation.field")?;
            let mut fm = Map::new();
            fm.insert(
                "from".into(),
                json!(safe_text(fo.get("from"), "relation.field.from", 500)?),
            );
            fm.insert(
                "to".into(),
                json!(safe_text(fo.get("to"), "relation.field.to", 500)?),
            );
            fields.push(Value::Object(fm));
        }
        rm.insert("fields".into(), Value::Array(fields));
        if let Some(v) = ro.get("enforced") {
            rm.insert(
                "enforced".into(),
                json!(req_bool(Some(v), "relation.enforced")?),
            );
        }
        relations_out.push(Value::Object(rm));
    }

    let limitation_values: &[Value] = match obj.get("limitations") {
        None => &[],
        Some(v) => as_array(Some(v), "limitations", 5000)?,
    };
    let mut limitations_out = Vec::new();
    for l in limitation_values {
        limitations_out.push(issue_value(Some(l), "limitation")?);
    }

    let mut out = Map::new();
    out.insert("schemaVersion".into(), json!(1));
    out.insert("source".into(), Value::Object(source));
    out.insert("assets".into(), Value::Array(assets_out));
    out.insert("relations".into(), Value::Array(relations_out));
    out.insert("limitations".into(), Value::Array(limitations_out));
    Ok(Value::Object(out))
}

fn is_external(a: &Value) -> bool {
    a.get("isLinked").and_then(Value::as_bool) == Some(true)
        || a.get("linked").and_then(Value::as_bool) == Some(true)
        || a.get("local").and_then(Value::as_bool) == Some(false)
        || a.get("linkedDatabaseName").is_some()
        || a.get("linkedTableName").is_some()
}

pub fn merge_inventories(inputs: &[Value]) -> Result<Value, String> {
    if inputs.len() > 20 {
        return Err("inventoriesは20件以下の配列を指定してください。".into());
    }
    let inventories: Vec<Value> = inputs
        .iter()
        .map(normalize_inventory)
        .collect::<Result<Vec<_>, _>>()?;
    if inventories.is_empty() {
        return Err("統合するInventoryを1件以上指定してください。".into());
    }
    let remap = |index: usize, id: &str| format!("db{}:{}", index + 1, id);
    let mut assets: Vec<Value> = Vec::new();
    let mut relations: Vec<Value> = Vec::new();
    let mut limitations: Vec<Value> = Vec::new();
    let mut mapped_indices: Vec<HashMap<String, usize>> = Vec::new();
    for (index, inv) in inventories.iter().enumerate() {
        let source_name = inv["source"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut mapped: HashMap<String, usize> = HashMap::new();
        for asset in inv["assets"].as_array().unwrap() {
            let orig_id = asset["id"].as_str().unwrap_or_default().to_string();
            let mut m = asset.as_object().unwrap().clone();
            m.insert("id".into(), json!(remap(index, &orig_id)));
            let deps: Vec<Value> = asset["dependsOn"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| json!(remap(index, d.as_str().unwrap_or_default())))
                .collect();
            m.insert("dependsOn".into(), Value::Array(deps));
            let issues: Vec<Value> = asset["issues"]
                .as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    let mut im = it.as_object().unwrap().clone();
                    if let Some(aid) = it.get("assetId") {
                        im.insert(
                            "assetId".into(),
                            json!(remap(index, aid.as_str().unwrap_or_default())),
                        );
                    }
                    Value::Object(im)
                })
                .collect();
            m.insert("issues".into(), Value::Array(issues));
            let caption_base = asset
                .get("caption")
                .and_then(Value::as_str)
                .unwrap_or_else(|| asset["name"].as_str().unwrap_or_default());
            m.insert(
                "caption".into(),
                json!(format!("{caption_base}【{}: {source_name}】", index + 1)),
            );
            mapped.insert(orig_id, assets.len());
            assets.push(Value::Object(m));
        }
        mapped_indices.push(mapped);
        for rel in inv["relations"].as_array().unwrap() {
            let mut rm = rel.as_object().unwrap().clone();
            rm.insert(
                "from".into(),
                json!(remap(index, rel["from"].as_str().unwrap_or_default())),
            );
            rm.insert(
                "to".into(),
                json!(remap(index, rel["to"].as_str().unwrap_or_default())),
            );
            relations.push(Value::Object(rm));
        }
        for lim in inv["limitations"].as_array().unwrap() {
            let mut lm = lim.as_object().unwrap().clone();
            if let Some(aid) = lim.get("assetId") {
                lm.insert(
                    "assetId".into(),
                    json!(remap(index, aid.as_str().unwrap_or_default())),
                );
            }
            limitations.push(Value::Object(lm));
        }
    }
    for (index, inv) in inventories.iter().enumerate() {
        for original in inv["assets"].as_array().unwrap() {
            if original["kind"] != json!("table") || !is_external(original) {
                continue;
            }
            let alias_idx = mapped_indices[index][original["id"].as_str().unwrap_or_default()];
            let alias_name = assets[alias_idx]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let alias_id = assets[alias_idx]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let add = |limitations: &mut Vec<Value>, code: &str, message: String| {
                let mut lm = Map::new();
                lm.insert("code".into(), json!(code));
                lm.insert("assetId".into(), json!(alias_id.clone()));
                lm.insert("message".into(), json!(message));
                limitations.push(Value::Object(lm));
            };
            let (Some(db), Some(tbl)) = (
                original.get("linkedDatabaseName").and_then(Value::as_str),
                original.get("linkedTableName").and_then(Value::as_str),
            ) else {
                add(
                    &mut limitations,
                    "LINK_METADATA_MISSING",
                    format!("{alias_name} のリンク先DB名またはテーブル名が未取得です。名前だけで統合していません。"),
                );
                continue;
            };
            let databases: Vec<usize> = inventories
                .iter()
                .enumerate()
                .filter(|(_, inv)| inv["source"]["name"].as_str() == Some(db))
                .map(|(i, _)| i)
                .collect();
            if databases.len() != 1 {
                let (code, word) = if databases.is_empty() {
                    ("LINK_BACKEND_MISSING", "提供されていない")
                } else {
                    ("LINK_AMBIGUOUS", "同名素材が複数あり対応が曖昧")
                };
                add(
                    &mut limitations,
                    code,
                    format!("{alias_name} のリンク先 {db} は{word}ため保留です。外部データは取得していません。"),
                );
                continue;
            }
            let candidate_index = databases[0];
            let tables: Vec<&Value> = inventories[candidate_index]["assets"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|a| {
                    a["kind"] == json!("table")
                        && !is_external(a)
                        && a["name"].as_str() == Some(tbl)
                })
                .collect();
            if tables.len() != 1 {
                let (code, word) = if tables.is_empty() {
                    ("LINK_TABLE_MISSING", "ローカルテーブルが未取得")
                } else {
                    ("LINK_AMBIGUOUS", "同名テーブルが複数")
                };
                add(
                    &mut limitations,
                    code,
                    format!("{alias_name} のリンク先 {db} 内の {tbl} は{word}のため保留です。"),
                );
                continue;
            }
            let target_idx =
                mapped_indices[candidate_index][tables[0]["id"].as_str().unwrap_or_default()];
            let target_id = assets[target_idx]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let target_fields = assets[target_idx].get("fields").cloned();
            let mut new_deps = assets[alias_idx]["dependsOn"].as_array().unwrap().clone();
            if !new_deps
                .iter()
                .any(|d| d.as_str() == Some(target_id.as_str()))
            {
                new_deps.push(json!(target_id));
            }
            let alias_map = assets[alias_idx].as_object_mut().unwrap();
            alias_map.insert("dependsOn".into(), Value::Array(new_deps));
            if let Some(fields) = target_fields {
                alias_map.insert("fields".into(), fields);
            }
            alias_map.remove("rowCount");
        }
    }
    let mut local_order: Vec<String> = Vec::new();
    let mut local_tables: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, inv) in inventories.iter().enumerate() {
        for asset in inv["assets"].as_array().unwrap() {
            if asset["kind"] != json!("table") || is_external(asset) {
                continue;
            }
            let name = asset["name"].as_str().unwrap_or_default().to_string();
            if !local_tables.contains_key(&name) {
                local_order.push(name.clone());
            }
            local_tables.entry(name).or_default().push(index);
        }
    }
    for name in local_order {
        let indexes = &local_tables[&name];
        if indexes.iter().collect::<HashSet<_>>().len() > 1 {
            limitations.push(json!({
                "code": "TABLE_NAME_COLLISION",
                "message": format!("{name} は別DBに同名テーブルがあります。明示リンクを除き別資産として保持し、同一テーブルかの判断は保留です。")
            }));
        }
    }
    let joined_names = inventories
        .iter()
        .map(|inv| inv["source"]["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" + ");
    let kinds_text = inventories
        .iter()
        .map(|inv| {
            format!(
                "{}: {}",
                inv["source"]["name"].as_str().unwrap_or_default(),
                inv["source"]["kind"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("、");
    limitations.push(json!({
        "code": "MERGED_METADATA_ONLY",
        "message": format!("アップロード順の最初の素材 {} を主DBとして、提供されたメタデータだけを統合しました。素材の出典種別は {kinds_text} です。実Accessの取得・動作・外部データ・業務再現はこの統合では検証していません。", inventories[0]["source"]["name"].as_str().unwrap_or_default())
    }));
    normalize_inventory(&json!({
        "schemaVersion": 1,
        "source": { "name": joined_names, "kind": "inventory" },
        "assets": assets,
        "relations": relations,
        "limitations": limitations
    }))
}

#[derive(Default)]
struct IssueSink {
    items: Vec<Value>,
    seen: HashSet<(String, Option<String>, Option<String>)>,
}

impl IssueSink {
    fn add(
        &mut self,
        code: &str,
        asset_id: Option<&str>,
        reference_id: Option<&str>,
        message: &str,
    ) {
        self.add_inner(code, asset_id, reference_id, message, false)
    }
    fn add_message_first(
        &mut self,
        code: &str,
        asset_id: Option<&str>,
        reference_id: Option<&str>,
        message: &str,
    ) {
        self.add_inner(code, asset_id, reference_id, message, true)
    }
    fn add_inner(
        &mut self,
        code: &str,
        asset_id: Option<&str>,
        reference_id: Option<&str>,
        message: &str,
        message_first: bool,
    ) {
        let key = (
            code.to_string(),
            asset_id.map(str::to_string),
            reference_id.map(str::to_string),
        );
        if !self.seen.insert(key) {
            return;
        }
        let mut m = Map::new();
        m.insert("code".into(), json!(code));
        if message_first {
            m.insert("message".into(), json!(message));
            if let Some(a) = asset_id {
                m.insert("assetId".into(), json!(a));
            }
            if let Some(r) = reference_id {
                m.insert("referenceId".into(), json!(r));
            }
        } else {
            if let Some(a) = asset_id {
                m.insert("assetId".into(), json!(a));
            }
            if let Some(r) = reference_id {
                m.insert("referenceId".into(), json!(r));
            }
            m.insert("message".into(), json!(message));
        }
        self.items.push(Value::Object(m));
    }
}

fn closure(
    edges: &HashMap<String, Vec<String>>,
    by_id: &HashMap<&str, &Value>,
    roots: &[String],
    report: bool,
    sink: &mut IssueSink,
) -> Vec<String> {
    enum Frame {
        Enter { id: String, from: Option<String> },
        Exit { id: String },
    }
    let mut stack: Vec<Frame> = roots
        .iter()
        .map(|id| Frame::Enter {
            id: id.clone(),
            from: None,
        })
        .collect();
    let mut visited: Vec<String> = Vec::new();
    let mut visited_set: HashSet<String> = HashSet::new();
    let mut active: HashSet<String> = HashSet::new();
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Exit { id } => {
                active.remove(&id);
            }
            Frame::Enter { id, from } => {
                if !by_id.contains_key(id.as_str()) {
                    if report {
                        sink.add(
                            "MISSING_REFERENCE",
                            from.as_deref(),
                            Some(&id),
                            &format!("参照先 {id} が未取得です。依存の範囲を確認してください。"),
                        );
                    }
                    continue;
                }
                if active.contains(&id) {
                    if report {
                        let f = from.as_deref().unwrap_or("undefined");
                        sink.add(
                            "CYCLE",
                            from.as_deref(),
                            Some(&id),
                            &format!("{f} と {id} を含む循環があります。移行順序と実行条件を確認してください。"),
                        );
                    }
                    continue;
                }
                if visited_set.contains(&id) {
                    continue;
                }
                visited_set.insert(id.clone());
                visited.push(id.clone());
                active.insert(id.clone());
                stack.push(Frame::Exit { id: id.clone() });
                if let Some(deps) = edges.get(&id) {
                    for dep in deps {
                        stack.push(Frame::Enter {
                            id: dep.clone(),
                            from: Some(id.clone()),
                        });
                    }
                }
            }
        }
    }
    visited
}

pub fn analyze_selection(input: &Value, selected_ids: &[String]) -> Result<Value, String> {
    let inventory = normalize_inventory(input)?;
    if selected_ids.len() > 5000 {
        return Err("selectedIdsは5000件以下の配列を指定してください。".into());
    }
    let mut selected: Vec<String> = Vec::new();
    {
        let mut seen = HashSet::new();
        for v in selected_ids {
            let id = safe_id_str(v, "selectedIds")?;
            if seen.insert(id.clone()) {
                selected.push(id);
            }
        }
    }
    if selected.is_empty() {
        return Err("フォーム・ページ・帳票を1件以上選択してください。".into());
    }
    let assets = inventory["assets"].as_array().unwrap();
    let by_id: HashMap<&str, &Value> = assets
        .iter()
        .map(|a| (a["id"].as_str().unwrap_or_default(), a))
        .collect();
    for id in &selected {
        let ok = by_id
            .get(id.as_str())
            .map(|a| matches!(a["kind"].as_str(), Some(k) if SELECTION_KINDS.contains(&k)))
            .unwrap_or(false);
        if !ok {
            return Err(format!(
                "選択対象は取得済みのフォーム・ページ・帳票にしてください: {id}"
            ));
        }
    }
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for a in assets {
        edges.insert(
            a["id"].as_str().unwrap_or_default().to_string(),
            a["dependsOn"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d.as_str().unwrap_or_default().to_string())
                .collect(),
        );
    }
    for r in inventory["relations"].as_array().unwrap() {
        let from = r["from"].as_str().unwrap_or_default();
        if let Some(v) = edges.get_mut(from) {
            v.push(r["to"].as_str().unwrap_or_default().to_string());
        }
    }
    let mut unresolved = IssueSink::default();
    let mut blockers = IssueSink::default();
    let included = closure(&edges, &by_id, &selected, true, &mut unresolved);
    let included_set: HashSet<String> = included.iter().cloned().collect();
    for id in &included {
        let asset = by_id[id.as_str()];
        let name = asset["name"].as_str().unwrap_or_default();
        let status = asset["status"].as_str().unwrap_or_default();
        if status != "supported" {
            if status == "unsupported" {
                blockers.add(
                    "UNSUPPORTED",
                    Some(id),
                    None,
                    &format!("{name} は非対応です。再現できると判断する前に確認が必要です。"),
                );
            } else {
                unresolved.add(
                    "PARTIAL",
                    Some(id),
                    None,
                    &format!("{name} は部分取得です。再現できると判断する前に確認が必要です。"),
                );
            }
        }
        if asset["kind"] == json!("external") || is_external(asset) {
            unresolved.add(
                "EXTERNAL_REFERENCE",
                Some(id),
                None,
                &format!("{name} は外部参照です。外部データの取得・更新・移行は行っていません。"),
            );
        }
        for item in asset["issues"].as_array().unwrap() {
            unresolved.add_message_first(
                item["code"].as_str().unwrap_or_default(),
                Some(id),
                item.get("referenceId").and_then(Value::as_str),
                item["message"].as_str().unwrap_or_default(),
            );
        }
    }
    for item in inventory["limitations"].as_array().unwrap() {
        let asset_id = item.get("assetId").and_then(Value::as_str);
        let keep = match asset_id {
            None => true,
            Some(a) => included_set.contains(a) || !by_id.contains_key(a),
        };
        if keep {
            unresolved.add_message_first(
                item["code"].as_str().unwrap_or_default(),
                asset_id,
                None,
                item["message"].as_str().unwrap_or_default(),
            );
        }
    }
    let mut impacted_ids: Vec<String> = Vec::new();
    let mut shared: Vec<String> = Vec::new();
    {
        let mut shared_set: HashSet<String> = HashSet::new();
        let mut dummy = IssueSink::default();
        for asset in assets {
            let id = asset["id"].as_str().unwrap_or_default();
            let is_screen =
                matches!(asset["kind"].as_str(), Some(k) if SELECTION_KINDS.contains(&k));
            if !is_screen || selected.iter().any(|s| s == id) {
                continue;
            }
            let visited = closure(&edges, &by_id, &[id.to_string()], false, &mut dummy);
            let mut has_overlap = false;
            for v in visited {
                if v != id && included_set.contains(&v) {
                    has_overlap = true;
                    if shared_set.insert(v.clone()) {
                        shared.push(v);
                    }
                }
            }
            if has_overlap {
                impacted_ids.push(id.to_string());
            }
        }
    }
    let dependency_ids: Vec<String> = included
        .into_iter()
        .filter(|id| !selected.contains(id))
        .collect();
    let mut evidence = vec![
        format!(
            "選択対象: {}。選択を自動追加していません。",
            selected.join("、")
        ),
        format!(
            "dependsOnとテーブル間の参照関係から、選択対象以外の依存を{}件確認しました。",
            dependency_ids.len()
        ),
    ];
    for id in &impacted_ids {
        evidence.push(format!(
            "未選択の {id} は選択範囲と共有する資産に依存します。変更時の影響確認が必要です。"
        ));
    }
    evidence.push("取得されたメタデータに基づく結果です。動的参照や未取得定義の依存が網羅されている保証はありません。".into());

    let mut out = Map::new();
    out.insert("selectedIds".into(), json!(selected));
    out.insert("dependencyIds".into(), json!(dependency_ids));
    out.insert("impactedIds".into(), json!(impacted_ids));
    out.insert("sharedIds".into(), json!(shared));
    out.insert("unresolved".into(), Value::Array(unresolved.items));
    out.insert("blockers".into(), Value::Array(blockers.items));
    out.insert("evidence".into(), json!(evidence));
    Ok(Value::Object(out))
}

fn normalize_usage(input: &Value) -> Result<Value, String> {
    let obj = as_object(Some(input), "usage")?;
    let mut m = Map::new();
    for key in USAGE_KEYS {
        let value = match obj.get(key) {
            None | Some(Value::Null) => Value::Null,
            Some(v) => match usage_choices(key) {
                Some(choices) => json!(choice(Some(v), choices, &format!("usage.{key}"))?),
                None => json!(req_bool(Some(v), &format!("usage.{key}"))?),
            },
        };
        m.insert(key.to_string(), value);
    }
    Ok(Value::Object(m))
}

fn usage_name(key: &str, value: &Value) -> String {
    if value.is_null() {
        return "未確認".into();
    }
    match (key, value) {
        ("offlineRequired", Value::Bool(b)) => if *b { "必要" } else { "不要" }.to_string(),
        ("concurrentEditing", Value::Bool(b)) => if *b { "あり" } else { "なし" }.to_string(),
        ("users", Value::String(s)) => match s.as_str() {
            "solo" => "一人".into(),
            "team" => "複数人".into(),
            _ => s.clone(),
        },
        ("location", Value::String(s)) => match s.as_str() {
            "device" => "同じPC".into(),
            "lan" => "社内の複数PC".into(),
            "remote" => "社外・遠隔".into(),
            _ => s.clone(),
        },
        ("permissions", Value::String(s)) => match s.as_str() {
            "same" => "同じ権限".into(),
            "roles" => "役割別".into(),
            _ => s.clone(),
        },
        ("coexistence", Value::String(s)) => match s.as_str() {
            "undecided" => "未定".into(),
            "keep-source" => "現行Accessを維持".into(),
            "shared-store" => "共有データで共存".into(),
            "replace-scope" => "選択範囲を切替".into(),
            _ => s.clone(),
        },
        (_, v) => v.to_string(),
    }
}

pub fn recommend_targets(
    input: &Value,
    input_usage: &Value,
    analysis: &Value,
) -> Result<Value, String> {
    normalize_inventory(input)?;
    let usage = normalize_usage(input_usage)?;
    let get = |k: &str| usage.get(k).cloned().unwrap_or(Value::Null);
    let unknowns: Vec<String> = USAGE_KEYS
        .iter()
        .filter(|k| {
            let v = get(k);
            v.is_null() || v.as_str() == Some("undecided")
        })
        .map(|k| format!("{}が未確認です。", usage_label(k)))
        .collect();
    let multi = get("users") == json!("team") || get("concurrentEditing") == json!(true);
    let complex =
        multi || get("location") == json!("remote") || get("permissions") == json!("roles");
    let mut constraints: Vec<String> = Vec::new();
    if get("coexistence") == json!("shared-store") {
        constraints.push(
            "既存Accessとの共有保存先、書込み責任、同期、競合回避を設計して検証する。".into(),
        );
    }
    if get("coexistence") == json!("keep-source") {
        constraints
            .push("既存Accessを残し、移行範囲との境界、二重入力、データ同期を確認する。".into());
    }
    let has_impacted = analysis
        .get("impactedIds")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if has_impacted {
        constraints.push("共有資産に依存する未選択の機能を変更前後に検証する。".into());
    }
    let has_unknown = !unknowns.is_empty();
    let offline = get("offlineRequired");

    let mut candidates: Vec<Map<String, Value>> = Vec::new();
    let mut web = Map::new();
    web.insert("id".into(), json!("web"));
    web.insert("label".into(), json!("Webアプリ"));
    web.insert(
        "fit".into(),
        json!(if complex {
            "recommended"
        } else {
            "conditional"
        }),
    );
    web.insert(
        "reasons".into(),
        json!([if complex {
            "複数利用者・遠隔利用・役割別権限の管理を設計しやすい候補です。"
        } else {
            "個人端末中心の利用には運用基盤の負担も比較する必要があります。"
        }]),
    );
    web.insert(
        "requirements".into(),
        json!(["認証・認可、DB、バックアップ、運用担当、配備先を具体化する。"]),
    );
    web.insert("unknowns".into(), json!(unknowns));
    candidates.push(web);

    let mut excel = Map::new();
    excel.insert("id".into(), json!("excel"));
    excel.insert("label".into(), json!("Excel"));
    let excel_fit = if complex {
        "not-recommended"
    } else if get("users") == json!("solo") && get("location") == json!("device") {
        "recommended"
    } else {
        "conditional"
    };
    excel.insert("fit".into(), json!(excel_fit));
    excel.insert(
        "reasons".into(),
        json!([if complex {
            "同時編集・遠隔利用・役割別権限があると、ファイル運用と競合管理の負担が大きくなります。"
        } else {
            "個人端末での表形式作業とオフライン利用に合う可能性があります。"
        }]),
    );
    excel.insert(
        "requirements".into(),
        json!([
            "フォーム、クエリ、帳票、VBAの代替方法とExcelでの再現範囲を確認する。",
            "一意性、参照整合性、同時書込み、バックアップを検証する。"
        ]),
    );
    excel.insert("unknowns".into(), json!(unknowns));
    candidates.push(excel);

    let mut gas = Map::new();
    gas.insert("id".into(), json!("sheets-gas"));
    gas.insert("label".into(), json!("Googleスプレッドシート + GAS"));
    let gas_fit = if offline == json!(true) {
        "not-recommended"
    } else if get("permissions") == json!("roles") {
        "conditional"
    } else if multi && get("location") == json!("remote") {
        "recommended"
    } else {
        "conditional"
    };
    gas.insert("fit".into(), json!(gas_fit));
    gas.insert(
        "reasons".into(),
        json!([if offline == json!(true) {
            "GASによる業務処理をオフラインで実行できる前提にはできません。"
        } else {
            "オンラインの共同編集に合う可能性があります。権限・データ量・実行制限の確認が必要です。"
        }]),
    );
    gas.insert(
        "requirements".into(),
        json!([
            "アカウント、情報管理方針、通信環境、GASの実行制限を確認する。",
            "行や役割別の権限が必要なら、シート共有だけで満たせると判断せず代替設計を検証する。"
        ]),
    );
    gas.insert("unknowns".into(), json!(unknowns));
    candidates.push(gas);

    for c in &mut candidates {
        if has_unknown && c.get("fit").and_then(Value::as_str) == Some("recommended") {
            c.insert("fit".into(), json!("conditional"));
        }
        if c.get("id").and_then(Value::as_str) == Some("web") && offline == json!(true) {
            c.insert("fit".into(), json!("conditional"));
            c.get_mut("requirements")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(json!(
                    "オフライン時の保存、再接続時の同期、競合解決を別途設計する。"
                ));
            c.get_mut("reasons")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(json!("オフライン対応の実装と検証が必要です。"));
        }
        for x in &constraints {
            c.get_mut("requirements")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(json!(x));
        }
        c.get_mut("reasons")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(json!(
                "ローカル規則による相対的な計画候補です。互換性・性能・適合は未実証です。"
            ));
    }
    Ok(Value::Array(
        candidates.into_iter().map(Value::Object).collect(),
    ))
}

fn push_requirement(
    requirements: &mut Vec<Value>,
    title: String,
    description: &str,
    evidence: Vec<String>,
    verification: &str,
) {
    let mut r = Map::new();
    r.insert(
        "id".into(),
        json!(format!("REQ-{:03}", requirements.len() + 1)),
    );
    r.insert("title".into(), json!(title));
    r.insert("description".into(), json!(description));
    r.insert("evidence".into(), json!(evidence));
    r.insert("verification".into(), json!(verification));
    r.insert("confirmed".into(), json!(false));
    requirements.push(Value::Object(r));
}

fn mermaid_text(v: &str) -> String {
    v.replace('"', "#quot;")
        .replace(|c: char| c == '\r' || c == '\n' || c == '`', " ")
}

fn build_diagrams(inventory: &Value, analysis: &Value) -> Value {
    let assets = inventory["assets"].as_array().unwrap();
    let by_id: HashMap<&str, &Value> = assets
        .iter()
        .map(|a| (a["id"].as_str().unwrap_or_default(), a))
        .collect();
    let mut included: Vec<String> = Vec::new();
    {
        let mut seen = HashSet::new();
        for key in ["selectedIds", "dependencyIds"] {
            for id in strings(&analysis[key]) {
                if seen.insert(id.clone()) {
                    included.push(id);
                }
            }
        }
    }
    let included_set: HashSet<String> = included.iter().cloned().collect();
    let impacted: Vec<String> = strings(&analysis["impactedIds"]);

    struct Namer {
        ids: Vec<(String, String)>,
        map: HashMap<String, String>,
    }
    impl Namer {
        fn node(&mut self, id: &str) -> String {
            if let Some(k) = self.map.get(id) {
                return k.clone();
            }
            let key = format!("n{}", self.ids.len() + 1);
            self.ids.push((id.to_string(), key.clone()));
            self.map.insert(id.to_string(), key.clone());
            key
        }
        fn get(&self, id: &str) -> String {
            self.map.get(id).cloned().unwrap_or_default()
        }
    }
    let mut namer = Namer {
        ids: Vec::new(),
        map: HashMap::new(),
    };
    let shape = |asset: &Value| -> String {
        let name = mermaid_text(asset["name"].as_str().unwrap_or_default());
        let kind = asset["kind"].as_str().unwrap_or_default();
        if kind == "table" {
            format!("[(\"table: {name}\")]")
        } else {
            format!("[\"{kind}: {name}\"]")
        }
    };

    let mut flow = vec!["flowchart LR".to_string()];
    let mut edges: Vec<String> = Vec::new();
    for id in &included {
        if let Some(asset) = by_id.get(id.as_str()) {
            for to in asset["dependsOn"].as_array().unwrap() {
                let to = to.as_str().unwrap_or_default();
                if included_set.contains(to) {
                    edges.push(format!("  {} --> {}", namer.node(id), namer.node(to)));
                }
            }
        }
    }
    let mut dotted: Vec<(String, String)> = Vec::new();
    let mut dotted_set: HashSet<(String, String)> = HashSet::new();
    for root in &impacted {
        let mut seen: HashSet<String> = HashSet::from([root.clone()]);
        let mut stack: Vec<Vec<String>> = vec![vec![root.clone()]];
        while let Some(path) = stack.pop() {
            let deps: Vec<String> = match path.last().and_then(|l| by_id.get(l.as_str())) {
                Some(asset) => strings(&asset["dependsOn"]),
                None => Vec::new(),
            };
            for to in deps {
                if included_set.contains(&to) {
                    let mut extended = path.clone();
                    extended.push(to);
                    for (index, id) in extended.iter().skip(1).enumerate() {
                        let pair = (path[index].clone(), id.clone());
                        if dotted_set.insert(pair.clone()) {
                            dotted.push(pair);
                        }
                    }
                } else if by_id.contains_key(to.as_str()) && !seen.contains(&to) {
                    seen.insert(to.clone());
                    let mut next = path.clone();
                    next.push(to);
                    stack.push(next);
                }
            }
        }
    }
    for (from, to) in dotted {
        let label = if included_set.contains(&to) {
            "|影響候補|"
        } else {
            ""
        };
        edges.push(format!(
            "  {} -.->{} {}",
            namer.node(&from),
            label,
            namer.node(&to)
        ));
    }
    for relation in inventory["relations"].as_array().unwrap() {
        let from = relation["from"].as_str().unwrap_or_default();
        let to = relation["to"].as_str().unwrap_or_default();
        if included_set.contains(from) && included_set.contains(to) {
            edges.push(format!(
                "  {} ---|関連| {}",
                namer.node(from),
                namer.node(to)
            ));
        }
    }
    for id in included.iter().chain(impacted.iter()) {
        namer.node(id);
    }
    for (id, key) in &namer.ids {
        let asset = by_id.get(id.as_str());
        if let Some(asset) = asset {
            flow.push(format!("  {key}{}", shape(asset)));
        }
    }
    flow.extend(edges);
    let selected_nodes: Vec<String> = strings(&analysis["selectedIds"])
        .iter()
        .map(|id| namer.get(id))
        .collect();
    flow.push("  classDef selected stroke-width:3px".into());
    flow.push(format!("  class {} selected", selected_nodes.join(",")));
    if !impacted.is_empty() {
        let impacted_nodes: Vec<String> = impacted.iter().map(|id| namer.get(id)).collect();
        flow.push("  classDef impacted stroke-dasharray:5 5".into());
        flow.push(format!("  class {} impacted", impacted_nodes.join(",")));
    }

    let tables: Vec<&Value> = assets
        .iter()
        .filter(|a| {
            a["kind"] == json!("table")
                && included_set.contains(a["id"].as_str().unwrap_or_default())
        })
        .collect();
    let table_ids: HashMap<&str, String> = tables
        .iter()
        .enumerate()
        .map(|(index, a)| {
            (
                a["id"].as_str().unwrap_or_default(),
                format!("t{}", index + 1),
            )
        })
        .collect();
    let mut er = if tables.is_empty() {
        Vec::new()
    } else {
        vec!["erDiagram".to_string()]
    };
    for asset in &tables {
        er.push(format!(
            "  {}[\"{}\"] {{",
            table_ids[asset["id"].as_str().unwrap_or_default()],
            mermaid_text(asset["name"].as_str().unwrap_or_default())
        ));
        if let Some(fields) = asset.get("fields").and_then(Value::as_array) {
            for (index, field) in fields.iter().enumerate() {
                let data_type = field.get("dataType").and_then(Value::as_str).unwrap_or("");
                let kind = if RE_ER_TYPE.is_match(data_type) {
                    data_type
                } else {
                    "field"
                };
                let pk = if field.get("isPrimaryKey").and_then(Value::as_bool) == Some(true) {
                    " PK"
                } else {
                    ""
                };
                er.push(format!(
                    "    {kind} f{}{pk} \"{}\"",
                    index + 1,
                    mermaid_text(field["name"].as_str().unwrap_or_default())
                ));
            }
        }
        er.push("  }".into());
    }
    for relation in inventory["relations"].as_array().unwrap() {
        let from = relation["from"].as_str().unwrap_or_default();
        let to = relation["to"].as_str().unwrap_or_default();
        if let (Some(f), Some(t)) = (table_ids.get(from), table_ids.get(to)) {
            let fields = relation["fields"].as_array().unwrap();
            let label = fields
                .iter()
                .map(|f| {
                    let from_field = f["from"].as_str().unwrap_or_default();
                    let to_field = f["to"].as_str().unwrap_or_default();
                    if to_field == from_field {
                        to_field.to_string()
                    } else {
                        format!("{to_field}={from_field}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let label = if label.is_empty() {
                "関連".to_string()
            } else {
                label
            };
            er.push(format!("  {t} ||--o{{ {f} : \"{}\"", mermaid_text(&label)));
        }
    }

    let mut out = Map::new();
    out.insert("flow".into(), json!(flow.join("\n")));
    out.insert("er".into(), json!(er.join("\n")));
    Value::Object(out)
}

pub fn build_plan(input: &Value, options: &Value) -> Result<Value, String> {
    let inventory = normalize_inventory(input)?;
    let options_obj = as_object(Some(options), "options")?;
    let selected: Vec<String> = as_array(options_obj.get("selectedIds"), "selectedIds", 5000)?
        .iter()
        .map(|v| safe_id(Some(v), "selectedIds"))
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyze_selection(&inventory, &selected)?;
    let empty_usage = json!({});
    let usage = normalize_usage(options_obj.get("usage").unwrap_or(&empty_usage))?;
    let get_usage = |k: &str| usage.get(k).cloned().unwrap_or(Value::Null);
    let candidates = recommend_targets(&inventory, &usage, &analysis)?;
    let candidates_arr = candidates.as_array().unwrap();
    let target = match options_obj.get("targetId") {
        None => candidates_arr
            .iter()
            .find(|c| c["fit"].as_str() == Some("recommended"))
            .or_else(|| candidates_arr.first())
            .cloned()
            .expect("候補は常に3つある"),
        Some(tid) => candidates_arr
            .iter()
            .find(|c| c["id"].as_str() == tid.as_str())
            .cloned()
            .ok_or("移行先候補が不正です。")?,
    };
    let notes = match options_obj.get("notes") {
        None => String::new(),
        Some(Value::String(s)) if s.is_empty() => String::new(),
        Some(v) => safe_text(Some(v), "notes", 4000)?,
    };

    let mut unresolved: Vec<Value> = analysis["unresolved"].as_array().unwrap().clone();
    unresolved.extend(analysis["blockers"].as_array().unwrap().iter().cloned());
    for message in target["unknowns"].as_array().unwrap() {
        unresolved.push(json!({ "code": "USAGE_UNKNOWN", "message": message }));
    }
    let shared: Vec<String> = strings(&analysis["sharedIds"]);
    let impacted: Vec<String> = strings(&analysis["impactedIds"]);
    if get_usage("coexistence") == json!("undecided")
        && (!shared.is_empty() || !impacted.is_empty())
    {
        unresolved.push(json!({ "code": "COEXISTENCE_UNDECIDED", "message": "共有資産と未選択機能への影響があるため、既存Accessとの共存方法と変更境界の確認が必要です。" }));
    }
    if target["fit"].as_str() == Some("not-recommended") {
        unresolved.push(json!({ "code": "TARGET_MISMATCH", "message": "選択した移行先と利用形態の制約を確認してください。" }));
    }

    let mut requirements: Vec<Value> = Vec::new();
    let assets = inventory["assets"].as_array().unwrap();
    for id in strings(&analysis["selectedIds"]) {
        let asset = assets
            .iter()
            .find(|a| a["id"].as_str() == Some(id.as_str()))
            .expect("選択資産は検証済み");
        let display = asset
            .get("caption")
            .and_then(Value::as_str)
            .unwrap_or_else(|| asset["name"].as_str().unwrap_or_default());
        push_requirement(
            &mut requirements,
            format!("{display}の業務を再現する"),
            "入力・表示・更新・例外処理・業務ルールを利用者と確認し、取得状態だけで再現可能と判断しない。",
            vec![
                format!("選択資産: {id}"),
                format!("取得状態: {}", asset["status"].as_str().unwrap_or_default()),
            ],
            "現行業務の操作シナリオと期待結果を用意し、利用者が移行先の結果を確認する。",
        );
    }
    let dependency_ids: Vec<String> = strings(&analysis["dependencyIds"]);
    let tables: Vec<&Value> = assets
        .iter()
        .filter(|a| {
            dependency_ids.contains(&a["id"].as_str().unwrap_or_default().to_string())
                && a["kind"] == json!("table")
        })
        .collect();
    if !tables.is_empty() {
        let evidence: Vec<String> = tables
            .iter()
            .map(|a| {
                let fields_len = a
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| f.len().to_string())
                    .unwrap_or_else(|| "未確認".into());
                let rows = match a.get("rowCount") {
                    None => "未計数".to_string(),
                    Some(v) => v.to_string(),
                };
                format!(
                    "{}: {fields_len}項目、件数{rows}",
                    a["id"].as_str().unwrap_or_default()
                )
            })
            .collect();
        push_requirement(
            &mut requirements,
            "データ構造と参照整合性を維持する".into(),
            "型、必須項目、主キー、関連、件数・内容の比較方法を定義する。行データの移行はこの計画生成では実行しない。",
            evidence,
            "許可されたコピーで主キー重複、必須値、関連、件数、代表レコード、業務集計を照合する。",
        );
    }
    let target_requirements: Vec<String> = target["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap_or_default().to_string())
        .collect();
    push_requirement(
        &mut requirements,
        "利用形態と移行先の制約を確認する".into(),
        &target_requirements.join(" "),
        USAGE_KEYS
            .iter()
            .map(|k| format!("{}: {}", usage_label(k), usage_name(k, &get_usage(k))))
            .collect(),
        "利用者、同時編集、通信断、権限、共存の各条件で操作とデータの整合性を確認する。",
    );
    if !impacted.is_empty() {
        push_requirement(
            &mut requirements,
            "未選択機能への影響を確認する".into(),
            "共有資産を変更する前に影響と変更境界を合意する。未選択機能を移行対象へ自動追加しない。",
            impacted.iter().map(|id| format!("影響候補: {id}")).collect(),
            "未選択フォーム・ページ・帳票を現行側で回帰確認し、結果と復旧条件を記録する。",
        );
    }
    if !unresolved.is_empty() {
        push_requirement(
            &mut requirements,
            "未解決事項を解消する".into(),
            "循環・動的参照・未取得・非対応・利用形態の不足を利用者と確認し、必要なら追加解析や個別設計を行う。",
            unresolved
                .iter()
                .map(|i| format!("{}: {}", i["code"].as_str().unwrap_or_default(), i["message"].as_str().unwrap_or_default()))
                .collect(),
            "各未解決事項に担当、確認方法、結果、残る制約を記録する。",
        );
    }

    let target_label = target["label"].as_str().unwrap_or_default();
    let migration_steps = vec![
        "1. 選択範囲・依存・影響候補・未解決事項を利用者と確認し、要件と変更境界の承認を得る。".to_string(),
        format!("2. {target_label}の試作方針、データ構造、画面・業務処理の代替方法、権限と利用環境を設計する。"),
        "3. 原本を保持し、許可されたコピーと合成データで移行手順・変換・検証を試行する。".to_string(),
        "4. 既存Accessとの共存、同期、切替時の書込み停止、バックアップ、復旧条件を定義する。".to_string(),
        "5. 要件を承認し検証結果を確認した後、別途承認された実装・移行作業で段階的に切り替える。".to_string(),
    ];
    let validation_steps = vec![
        "選択業務の正常系・入力異常・取消・更新・帳票結果を現行側の期待結果と比較する。"
            .to_string(),
        "依存データの型・必須値・主キー・参照整合性・件数・代表値・業務集計を比較する。"
            .to_string(),
        "未選択の共有DB利用機能を回帰確認し、共存中の両側の整合性を確認する。".to_string(),
        "利用人数・同時更新の競合・役割別アクセス・ネットワーク断と再接続を確認する。".to_string(),
        "バックアップからの復元と切戻しを試行し、利用者の受入結果と未解決事項を記録する。"
            .to_string(),
    ];
    let mut risks = vec![
        "取得状態がsupportedでも、移行先での業務再現は未検証です。".to_string(),
        "動的参照や未取得の定義により依存や影響候補が不足する可能性があります。".to_string(),
    ];
    if !impacted.is_empty() {
        risks.push("共有資産の変更は未選択の機能へ影響する可能性があります。".into());
    }
    let coexistence = get_usage("coexistence");
    if coexistence == json!("undecided") || coexistence.is_null() {
        risks.push("既存Accessとの共存方法が未定です。切替を確定できません。".into());
    }
    if coexistence == json!("shared-store") {
        risks.push(
            "Accessと移行先の同時書込みでは競合・同期・障害復旧の個別検証が必要です。".into(),
        );
    }
    let mut notices = vec![
        "この出力は移行計画の草案です。アプリ生成、データ移行、原本の更新、外部API呼出しを行っていません。".to_string(),
        "要件はすべて未承認です。移行先候補の適合性・性能・互換性は未実証です。".to_string(),
    ];
    match inventory["source"]["kind"].as_str().unwrap_or_default() {
        "synthetic" => notices.push("出典は完全な合成データです。実Accessの解析結果ではありません。".into()),
        "access" => notices.push("入力はAccess由来のメタデータとして渡されています。実ファイルの取得・動作はこの処理では確認していません。取得不能な定義・外部データは対応済みと扱いません。".into()),
        _ => notices.push("出典は入力されたInventoryメタデータです。実Accessでの取得・動作はこの処理では確認していません。".into()),
    }
    if !notes.is_empty() {
        notices.push(format!("利用者メモ: {notes}"));
    }

    let mut out = Map::new();
    out.insert("version".into(), json!(1));
    out.insert(
        "status".into(),
        json!(if unresolved.is_empty() {
            "draft"
        } else {
            "review-required"
        }),
    );
    out.insert("source".into(), inventory["source"].clone());
    out.insert("analysis".into(), analysis.clone());
    out.insert("usage".into(), usage);
    out.insert("target".into(), target.clone());
    out.insert("requirements".into(), Value::Array(requirements));
    out.insert("migrationSteps".into(), json!(migration_steps));
    out.insert("validationSteps".into(), json!(validation_steps));
    out.insert("risks".into(), json!(risks));
    out.insert("unresolved".into(), Value::Array(unresolved));
    out.insert("notices".into(), json!(notices));
    out.insert("diagrams".into(), build_diagrams(&inventory, &analysis));
    let mut hand_over = Map::new();
    hand_over.insert(
        "summary".into(),
        json!(
            "選択範囲から移行計画草案を生成しました。要件承認・実装・移行・受入検証は未実施です。"
        ),
    );
    hand_over.insert("selectedIds".into(), analysis["selectedIds"].clone());
    hand_over.insert("targetId".into(), target["id"].clone());
    hand_over.insert(
        "pending".into(),
        json!([
            "要件と変更境界の利用者承認",
            "未解決事項の確認",
            "実装・移行と受入検証"
        ]),
    );
    out.insert("hand_over".into(), Value::Object(hand_over));
    Ok(Value::Object(out))
}

fn markdown_escape(value: &str) -> String {
    let doubled = value.replace('\\', "\\\\");
    let mut escaped = String::with_capacity(doubled.len());
    for c in doubled.chars() {
        if matches!(
            c,
            '`' | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '#'
                | '+'
                | '.'
                | '!'
                | '|'
                | '>'
                | '~'
                | '-'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("\r\n", " ")
        .replace('\n', " ")
}

pub fn render_plan_markdown(plan: &Value) -> String {
    let empty = Map::new();
    let obj = plan.as_object().unwrap_or(&empty);
    let str_of = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let arr_of = |key: &str| -> Vec<String> { strings(obj.get(key).unwrap_or(&Value::Null)) };
    let status_name = match str_of("status").as_str() {
        "draft" => "草案".to_string(),
        "review-required" => "要確認".to_string(),
        other => other.to_string(),
    };
    let source = obj.get("source").unwrap_or(&Value::Null);
    let source_name = source
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let source_kind_name = match source
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "synthetic" => "合成サンプル".to_string(),
        "access" => "Access由来の構造資料".to_string(),
        "inventory" => "入力・統合された構造資料".to_string(),
        other => other.to_string(),
    };
    let target = obj.get("target").unwrap_or(&Value::Null);
    let fit_name = match target
        .get("fit")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "recommended" => "推薦".to_string(),
        "conditional" => "条件付き".to_string(),
        "not-recommended" => "要確認".to_string(),
        other => other.to_string(),
    };
    let analysis = obj.get("analysis").unwrap_or(&Value::Null);
    let join_md = |items: &[String], sep: &str| -> String {
        items
            .iter()
            .map(|s| markdown_escape(s))
            .collect::<Vec<_>>()
            .join(sep)
    };
    let usage = obj.get("usage").unwrap_or(&Value::Null);

    let mut lines: Vec<String> = Vec::new();
    lines.push("# 移行計画（草案）".into());
    lines.push(String::new());
    lines.push(format!(
        "状態: {} / 要件は未承認",
        markdown_escape(&status_name)
    ));
    lines.push(format!(
        "出典: {}（{}）",
        markdown_escape(&source_name),
        markdown_escape(&source_kind_name)
    ));
    lines.push(String::new());
    lines.push("## 注意事項".into());
    for text in arr_of("notices") {
        lines.push(format!("- {}", markdown_escape(&text)));
    }
    lines.push(String::new());
    lines.push("## 選択範囲と依存".into());
    lines.push(format!(
        "- 選択: {}",
        join_md(&strings(&analysis["selectedIds"]), "、")
    ));
    let deps = join_md(&strings(&analysis["dependencyIds"]), "、");
    lines.push(format!(
        "- 依存: {}",
        if deps.is_empty() {
            "取得された依存なし".into()
        } else {
            deps
        }
    ));
    let shared = join_md(&strings(&analysis["sharedIds"]), "、");
    lines.push(format!(
        "- 共有資産: {}",
        if shared.is_empty() {
            "確認された共有なし".into()
        } else {
            shared
        }
    ));
    let impacted = join_md(&strings(&analysis["impactedIds"]), "、");
    lines.push(format!(
        "- 未選択への影響候補: {}",
        if impacted.is_empty() {
            "取得済み定義からの候補なし".into()
        } else {
            impacted
        }
    ));
    lines.push(String::new());
    lines.push("### 根拠".into());
    for text in strings(&analysis["evidence"]) {
        lines.push(format!("- {}", markdown_escape(&text)));
    }
    lines.push(String::new());
    lines.push("## 利用形態".into());
    for key in USAGE_KEYS {
        let value = usage.get(key).cloned().unwrap_or(Value::Null);
        lines.push(format!(
            "- {}: {}",
            usage_label(key),
            markdown_escape(&usage_name(key, &value))
        ));
    }
    lines.push(String::new());
    lines.push("## 移行先候補".into());
    lines.push(format!(
        "{}（{}）",
        markdown_escape(
            target
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
        markdown_escape(&fit_name)
    ));
    for text in strings(target.get("reasons").unwrap_or(&Value::Null)) {
        lines.push(format!("- {}", markdown_escape(&text)));
    }
    lines.push(String::new());
    lines.push("## 要件（すべて未承認）".into());
    for requirement in obj
        .get("requirements")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let r = |key: &str| {
            requirement
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        lines.push(String::new());
        lines.push(format!(
            "### {} {}",
            markdown_escape(&r("id")),
            markdown_escape(&r("title"))
        ));
        lines.push(markdown_escape(&r("description")));
        lines.push(String::new());
        lines.push(format!(
            "根拠: {}",
            requirement
                .get("evidence")
                .and_then(Value::as_array)
                .map(|a| a
                    .iter()
                    .map(|e| markdown_escape(e.as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join(" / "))
                .unwrap_or_default()
        ));
        lines.push(format!("検証方法: {}", markdown_escape(&r("verification"))));
        lines.push("承認: 未承認".into());
    }
    for (title, key) in [
        ("移行計画", "migrationSteps"),
        ("検証計画", "validationSteps"),
        ("リスク", "risks"),
    ] {
        lines.push(String::new());
        lines.push(format!("## {title}"));
        for text in arr_of(key) {
            lines.push(format!("- {}", markdown_escape(&text)));
        }
    }
    if let Some(diagrams) = plan.get("diagrams").filter(|d| !d.is_null()) {
        lines.push(String::new());
        lines.push("## 図（Mermaid）".into());
        lines.push("取得済みの依存・関連だけから描いた候補です。動的参照や未取得の定義は含まれず、業務の手順そのものではありません。".into());
        lines.push(String::new());
        lines.push("### 業務フロー候補（選択対象の依存図）".into());
        lines.push("太線は選択対象、点線は影響を受ける未選択画面です。".into());
        lines.push(String::new());
        lines.push("```mermaid".into());
        lines.push(
            diagrams
                .get("flow")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        lines.push("```".into());
        lines.push(String::new());
        lines.push("### ER図（依存範囲のテーブル）".into());
        lines.push(String::new());
        let er = diagrams
            .get("er")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !er.is_empty() {
            lines.push("```mermaid".into());
            lines.push(er);
            lines.push("```".into());
        } else {
            lines.push("依存範囲にテーブルはありません。".into());
        }
    }
    lines.push(String::new());
    lines.push("## 未解決事項".into());
    let unresolved = obj.get("unresolved").and_then(Value::as_array);
    match unresolved {
        Some(items) if !items.is_empty() => {
            for item in items {
                let suffix = match item.get("assetId").and_then(Value::as_str) {
                    Some(a) => format!("（{}）", markdown_escape(a)),
                    None => String::new(),
                };
                lines.push(format!(
                    "- {}: {}{}",
                    markdown_escape(item.get("code").and_then(Value::as_str).unwrap_or_default()),
                    markdown_escape(
                        item.get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                    suffix
                ));
            }
        }
        _ => lines.push(
            "- 取得済み情報からの指摘なし。要件・適合・動作の承認と検証は未実施です。".into(),
        ),
    }
    lines.push(String::new());
    lines.push("## 引き継ぎ".into());
    let hand_over = obj.get("hand_over").unwrap_or(&Value::Null);
    lines.push(markdown_escape(
        hand_over
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ));
    for text in strings(hand_over.get("pending").unwrap_or(&Value::Null)) {
        lines.push(format!("- {}", markdown_escape(&text)));
    }
    lines.push(String::new());
    lines.join("\n")
}
