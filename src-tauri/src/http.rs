use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

use crate::access::{AccessFailure, AccessHelper, WorkerProgress};

static PUBLIC: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../public");
static SAMPLES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../samples");
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState(Arc<StateInner>);

struct StateInner {
    port: u16,
    token: String,
    helper: AccessHelper,
    access_cache: Mutex<Option<(Instant, Value)>>,
    active_import: Mutex<Option<Arc<ImportJob>>>,
    last_import: Mutex<Option<ImportSnapshot>>,
    last_import_error: Mutex<Option<(String, AccessFailure)>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportSnapshot {
    request_id: String,
    state: String,
    phase: String,
    completed: u64,
    total: Option<u64>,
}

struct ImportJob {
    request_id: String,
    snapshot: Mutex<ImportSnapshot>,
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
    result: Mutex<Option<Result<Value, AccessFailure>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressQuery {
    request_id: String,
}

struct CancelOnDrop(Option<watch::Sender<bool>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(true);
        }
    }
}

impl AppState {
    pub fn new(port: u16, token: String, script_dir: PathBuf) -> Self {
        Self(Arc::new(StateInner {
            port,
            token,
            helper: AccessHelper { script_dir },
            access_cache: Mutex::new(None),
            active_import: Mutex::new(None),
            last_import: Mutex::new(None),
            last_import_error: Mutex::new(None),
        }))
    }

    pub fn import_is_active(&self) -> bool {
        self.0
            .active_import
            .try_lock()
            .map(|active| active.is_some())
            .unwrap_or(true)
    }

    pub async fn cancel_active_import(&self) {
        let job = self.0.active_import.lock().await.clone();
        if let Some(job) = job {
            if job.result.lock().await.is_none() {
                let _ = job.cancel.send(true);
            }
            let _ = wait_for_job_result(&job).await;
        }
    }
}

async fn wait_for_job_result(job: &ImportJob) -> Option<Result<Value, AccessFailure>> {
    let mut done = job.done.subscribe();
    loop {
        let result = job.result.lock().await.clone();
        if result.is_some() {
            return result;
        }
        if *done.borrow() || done.changed().await.is_err() {
            return job.result.lock().await.clone();
        }
    }
}

#[derive(Debug)]
struct ApiError(StatusCode, &'static str, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response()
    }
}

fn bad_json() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "INVALID_JSON",
        "JSONファイルを読み取れません。形式を確認してください。",
    )
}

fn parse_json(bytes: &[u8], limit: usize) -> Result<Value, ApiError> {
    if bytes.len() > limit {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "INPUT_TOO_LARGE",
            "ファイルまたは入力データがサイズ上限を超えています。",
        ));
    }
    serde_json::from_slice(bytes).map_err(|_| bad_json())
}

fn checked_inventory(input: &Value) -> Result<Value, ApiError> {
    access2future_domain::normalize_inventory(input).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_INVENTORY",
            "解析資料の版、資産ID、項目の型を確認してください。",
        )
    })
}

async fn local_security(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let expected_host = format!("127.0.0.1:{}", state.0.port);
    if request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        != Some(expected_host.as_str())
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "LOCAL_REQUEST_ONLY",
            "このアプリはローカル接続専用です。",
        ));
    }
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if origin != format!("http://{expected_host}") {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "FOREIGN_ORIGIN",
                "別のサイトからの要求は受け付けません。",
            ));
        }
    }
    if request.uri().path().starts_with("/api/")
        && request
            .headers()
            .get("x-a2f-token")
            .and_then(|value| value.to_str().ok())
            != Some(state.0.token.as_str())
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "INVALID_LOCAL_TOKEN",
            "この操作は受け付けません。",
        ));
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    Ok(response)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/demo", get(demo))
        .route("/api/inventory", post(inventory))
        .route("/api/merge", post(merge))
        .route("/api/analyze", post(analyze))
        .route("/api/plan", post(plan))
        .route("/api/import", post(import_access))
        .route("/api/import/progress", get(import_progress))
        .route("/api/import/cancel", post(cancel_import))
        .fallback(static_asset)
        .layer(DefaultBodyLimit::max(MAX_FILE_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_security,
        ))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let mut cache = state.0.access_cache.lock().await;
    if cache
        .as_ref()
        .is_none_or(|(when, _)| when.elapsed() >= Duration::from_secs(60))
    {
        *cache = Some((Instant::now(), state.0.helper.capabilities().await));
    }
    let access = cache
        .as_ref()
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| json!({"available":false}));
    Json(json!({"version":"0.1.0","mode":"local","access":access,"maxFileBytes":MAX_FILE_BYTES}))
}

async fn demo() -> Result<Json<Value>, ApiError> {
    let bytes = SAMPLES.get_file("demo.inventory.json").ok_or(ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "SAMPLE_UNAVAILABLE",
        "合成サンプルを読み取れません。",
    ))?;
    let mut value =
        checked_inventory(&serde_json::from_slice(bytes.contents()).map_err(|_| bad_json())?)?;
    value["source"]["kind"] = Value::String("synthetic".into());
    Ok(Json(value))
}

async fn inventory(headers: HeaderMap, bytes: Bytes) -> Result<Json<Value>, ApiError> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| !v.starts_with("application/json"))
    {
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "EXPECTED_JSON",
            "JSON形式の入力を使用してください。",
        ));
    }
    let value = parse_json(&bytes, MAX_JSON_BYTES)?;
    let mut normalized = checked_inventory(&value)?;
    normalized["source"]["kind"] = Value::String("inventory".into());
    Ok(Json(normalized))
}

async fn merge(headers: HeaderMap, bytes: Bytes) -> Result<Json<Value>, ApiError> {
    require_json(&headers)?;
    let body = parse_json(&bytes, MAX_JSON_BYTES)?;
    let inventories = body
        .get("inventories")
        .and_then(Value::as_array)
        .filter(|values| (2..=8).contains(&values.len()))
        .ok_or(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_INVENTORY_SET",
            "統合する解析資料を2〜8件指定してください。",
        ))?;
    let merged = access2future_domain::merge_inventories(inventories).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_INVENTORY_SET",
            "統合する資料の版と資産・リンク情報を確認してください。",
        )
    })?;
    Ok(Json(merged))
}

async fn analyze(headers: HeaderMap, bytes: Bytes) -> Result<Json<Value>, ApiError> {
    require_json(&headers)?;
    let body = parse_json(&bytes, MAX_JSON_BYTES)?;
    let inventory = checked_inventory(body.get("inventory").ok_or_else(bad_json)?)?;
    let selected: Vec<String> = body
        .get("selectedIds")
        .and_then(Value::as_array)
        .ok_or_else(bad_json)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(bad_json))
        .collect::<Result<_, _>>()?;
    let analysis =
        access2future_domain::analyze_selection(&inventory, &selected).map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "INVALID_SELECTION",
                "選択対象を確認してください。",
            )
        })?;
    let usage = body.get("usage").cloned().unwrap_or_else(|| json!({}));
    let targets =
        access2future_domain::recommend_targets(&inventory, &usage, &analysis).map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "INVALID_USAGE",
                "利用形態の入力を確認してください。",
            )
        })?;
    Ok(Json(json!({"analysis":analysis,"targets":targets})))
}

async fn plan(headers: HeaderMap, bytes: Bytes) -> Result<Json<Value>, ApiError> {
    require_json(&headers)?;
    let body = parse_json(&bytes, MAX_JSON_BYTES)?;
    let selected = body
        .get("selectedIds")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or(ApiError(
            StatusCode::BAD_REQUEST,
            "NO_SELECTION",
            "移行したいフォーム・ページ・帳票を一つ以上選択してください。",
        ))?;
    let inventory = checked_inventory(body.get("inventory").ok_or_else(bad_json)?)?;
    let selection = access2future_domain::analyze_selection(
        &inventory,
        &selected
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_PLAN_INPUT",
            "選択対象、利用形態、移行先の入力を確認してください。",
        )
    })?;
    let mut plan_input = json!({"selectedIds":selected,"usage":body.get("usage").cloned().unwrap_or_else(||json!({}))});
    if let Some(target) = body.get("targetId") {
        plan_input["targetId"] = target.clone();
    }
    if let Some(notes) = body.get("notes") {
        plan_input["notes"] = notes.clone();
    }
    let mut plan = access2future_domain::build_plan(&inventory, &plan_input).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_PLAN_INPUT",
            "選択対象、利用形態、移行先の入力を確認してください。",
        )
    })?;
    if plan.get("analysis").is_none() {
        plan["analysis"] = selection;
    }
    let markdown = access2future_domain::render_plan_markdown(&plan);
    Ok(Json(json!({"plan":plan,"markdown":markdown})))
}

fn require_json(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|value| !value.starts_with("application/json"))
    {
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "EXPECTED_JSON",
            "JSON形式の入力を使用してください。",
        ));
    }
    Ok(())
}

async fn static_asset(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ApiError> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Err(ApiError(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "この操作は受け付けません。",
        ));
    }
    let raw = request.uri().path();
    if raw.contains('\\')
        || raw.contains('\0')
        || raw.split('/').any(|segment| segment == "..")
        || raw.to_ascii_lowercase().contains("%2e")
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_PATH",
            "このパスにはアクセスできません。",
        ));
    }
    let path = if raw == "/" {
        "index.html"
    } else {
        raw.trim_start_matches('/')
    };
    let file = PUBLIC.get_file(path).ok_or(ApiError(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "要求されたファイルが見つかりません。",
    ))?;
    let mut bytes = file.contents().to_vec();
    if path == "index.html" {
        let html = String::from_utf8(bytes).map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ASSET_INVALID",
                "画面を読み取れません。",
            )
        })?;
        bytes = html
            .replace(
                "</head>",
                &format!(
                    "<meta name=\"access2future-token\" content=\"{}\"></head>",
                    state.0.token
                ),
            )
            .into_bytes();
    }
    let content_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();
    let mut response = Response::new(if request.method() == Method::HEAD {
        Body::empty()
    } else {
        Body::from(std::mem::take(&mut bytes))
    });
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&format!("{content_type}; charset=utf-8"))
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    Ok(response)
}

async fn import_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|value| !value.starts_with("application/octet-stream"))
    {
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "EXPECTED_ACCESS_FILE",
            "ファイル本体を送信してください。ローカルパスの指定は受け付けません。",
        ));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "INPUT_TOO_LARGE",
            "ファイルまたは入力データがサイズ上限を超えています。",
        ));
    }
    let encoded = headers
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let name = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "INVALID_FILE_NAME",
                "ファイル名を読み取れません。",
            )
        })?
        .into_owned();
    let extension = PathBuf::from(&name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.is_empty()
        || name.len() > 180
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
        || !matches!(extension.as_str(), "mdb" | "accdb")
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "UNSUPPORTED_FILE",
            "対応する .mdb または .accdb ファイルを選択してください.",
        ));
    }
    let request_id = headers
        .get("x-import-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&request_id).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_IMPORT_ID",
            "解析の識別情報を確認してください。",
        )
    })?;
    if bytes.len() < 128
        || !(bytes[4..20].starts_with(b"Standard Jet DB\0")
            || bytes[4..20].starts_with(b"Standard ACE DB\0"))
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_ACCESS_FILE",
            "Accessのデータベースとして確認できません。ファイル形式を確認してください。",
        ));
    }
    let mut active = state.0.active_import.lock().await;
    if active.is_some() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "IMPORT_BUSY",
            "別のファイルを解析しています。完了してから再度お試しください。",
        ));
    }
    let initial = ImportSnapshot {
        request_id: request_id.clone(),
        state: "running".into(),
        phase: "preparing".into(),
        completed: 0,
        total: None,
    };
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (done_tx, _) = watch::channel(false);
    let (progress_tx, mut progress_rx) = watch::channel(WorkerProgress {
        phase: "preparing".into(),
        completed: 0,
        total: None,
    });
    let job = Arc::new(ImportJob {
        request_id: request_id.clone(),
        snapshot: Mutex::new(initial.clone()),
        cancel: cancel_tx.clone(),
        done: done_tx,
        result: Mutex::new(None),
    });
    *active = Some(job.clone());
    drop(active);
    *state.0.last_import_error.lock().await = None;
    let mut guard = CancelOnDrop(Some(cancel_tx));
    let state_bg = state.clone();
    let job_bg = job.clone();
    let file_name = name;
    let payload = bytes.to_vec();
    tokio::spawn(async move {
        let job_progress = job_bg.clone();
        let watcher = tokio::spawn(async move {
            loop {
                if progress_rx.changed().await.is_err() {
                    break;
                }
                let progress = progress_rx.borrow().clone();
                update_progress(&job_progress, &progress).await;
            }
        });
        let result = state_bg
            .0
            .helper
            .import(&payload, &file_name, progress_tx, cancel_rx)
            .await;
        watcher.abort();
        let state_name = if result.is_ok() {
            "completed"
        } else if result
            .as_ref()
            .err()
            .is_some_and(|error| error.code == "ACCESS_CANCELLED")
        {
            "cancelled"
        } else {
            "failed"
        };
        {
            let mut snapshot = job_bg.snapshot.lock().await;
            snapshot.state = state_name.into();
            if let Ok(inventory) = &result {
                let count = inventory
                    .get("assets")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len) as u64;
                snapshot.phase = "finishing".into();
                snapshot.completed = count;
                snapshot.total = Some(count);
            }
            *state_bg.0.last_import.lock().await = Some(snapshot.clone());
        }
        *state_bg.0.last_import_error.lock().await = result
            .as_ref()
            .err()
            .cloned()
            .map(|error| (job_bg.request_id.clone(), error));
        *job_bg.result.lock().await = Some(result);
        let mut active = state_bg.0.active_import.lock().await;
        if active
            .as_ref()
            .is_some_and(|current| current.request_id == job_bg.request_id)
        {
            *active = None;
        }
        job_bg.done.send_replace(true);
    });
    let result = wait_for_job_result(&job).await.ok_or(ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "IMPORT_RESULT_MISSING",
        "解析結果を確認できませんでした。",
    ))?;
    guard.0 = None;
    result.map(Json).map_err(api_access_error)
}

async fn import_progress(
    State(state): State<AppState>,
    Query(query): Query<ProgressQuery>,
) -> Result<Json<Value>, ApiError> {
    Uuid::parse_str(&query.request_id).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_IMPORT_ID",
            "解析の識別情報を確認してください。",
        )
    })?;
    if let Some(job) = state
        .0
        .active_import
        .lock()
        .await
        .as_ref()
        .filter(|job| job.request_id == query.request_id)
        .cloned()
    {
        return Ok(Json(
            serde_json::to_value(job.snapshot.lock().await.clone()).unwrap_or_else(|_| json!({})),
        ));
    }
    if let Some(snapshot) = state
        .0
        .last_import
        .lock()
        .await
        .as_ref()
        .filter(|snapshot| snapshot.request_id == query.request_id)
    {
        return Ok(Json(
            serde_json::to_value(snapshot).unwrap_or_else(|_| json!({})),
        ));
    }
    Ok(Json(
        json!({"requestId":query.request_id,"state":"waiting","phase":"preparing","completed":0,"total":null}),
    ))
}

async fn cancel_import(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let request_id = headers
        .get("x-import-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if let Some(request_id) = &request_id {
        Uuid::parse_str(request_id).map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "INVALID_IMPORT_ID",
                "解析の識別情報を確認してください。",
            )
        })?;
    }
    let job = state.0.active_import.lock().await.clone();
    if let Some(job) = job {
        if request_id
            .as_deref()
            .is_some_and(|request_id| request_id != job.request_id)
        {
            return Ok(Json(json!({"cancelled":false})));
        }
        if job.result.lock().await.is_none() {
            let _ = job.cancel.send(true);
        }
        return match wait_for_job_result(&job).await {
            Some(Err(error)) if error.code == "ACCESS_CANCELLED" => {
                Ok(Json(json!({"cancelled":true})))
            }
            Some(Err(error)) => Err(api_access_error(error)),
            Some(Ok(_)) => Ok(Json(json!({"cancelled":false}))),
            None => Ok(Json(json!({"cancelled":false}))),
        };
    }
    if let Some(request_id) = request_id {
        let snapshot = state
            .0
            .last_import
            .lock()
            .await
            .clone()
            .filter(|snapshot| snapshot.request_id == request_id);
        if let Some(snapshot) = snapshot {
            if snapshot.state == "cancelled" {
                return Ok(Json(json!({"cancelled":true})));
            }
            if snapshot.state == "failed" {
                if let Some((error_id, error)) = state.0.last_import_error.lock().await.clone() {
                    if error_id == request_id {
                        if error.code == "ACCESS_CANCELLED" {
                            return Ok(Json(json!({"cancelled":true})));
                        }
                        return Err(api_access_error(error));
                    }
                }
                return Err(ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "IMPORT_FAILED",
                    "Accessの解析が失敗しました。ファイルと取得可能な範囲を確認してください。",
                ));
            }
        }
    }
    Ok(Json(json!({"cancelled":false})))
}

async fn update_progress(job: &ImportJob, progress: &WorkerProgress) {
    if !["preparing", "enumerating", "analysing", "finishing"].contains(&progress.phase.as_str())
        || progress
            .total
            .is_some_and(|total| progress.completed > total)
        || (progress.total.is_none() && progress.completed != 0)
    {
        return;
    }
    let mut snapshot = job.snapshot.lock().await;
    let phases = ["preparing", "enumerating", "analysing", "finishing"];
    let old = phases
        .iter()
        .position(|phase| *phase == snapshot.phase)
        .unwrap_or(0);
    let new = phases
        .iter()
        .position(|phase| *phase == progress.phase)
        .unwrap_or(0);
    if progress.completed < snapshot.completed
        || new < old
        || (snapshot.total.is_some() && progress.total != snapshot.total)
    {
        return;
    }
    snapshot.phase = progress.phase.clone();
    snapshot.completed = progress.completed;
    snapshot.total = progress.total;
}

fn api_access_error(error: AccessFailure) -> ApiError {
    ApiError(
        StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        error.code,
        error.message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn send(
        state: AppState,
        method: Method,
        path: &str,
        body: Value,
        token: bool,
    ) -> Response {
        let host = format!("127.0.0.1:{}", state.0.port);
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("http://{host}{path}"))
            .header(header::HOST, &host);
        if token {
            builder = builder.header("x-a2f-token", &state.0.token);
        }
        if path.starts_with("/api/") {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        router(state)
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    fn state() -> AppState {
        AppState::new(39177, "test-token".into(), PathBuf::new())
    }

    #[tokio::test]
    async fn desktop_bundles_the_local_mermaid_renderer_and_blob_image_policy() {
        let state = state();
        let page = send(state.clone(), Method::GET, "/", json!({}), false).await;
        assert_eq!(page.status(), StatusCode::OK);
        assert!(
            page.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("img-src 'self' data: blob:")
        );
        let html = page.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&html).contains("/vendor/mermaid.tiny.js"));

        let app_module = send(state.clone(), Method::GET, "/app.mjs", json!({}), false).await;
        assert_eq!(app_module.status(), StatusCode::OK);
        assert!(
            app_module.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("javascript")
        );
        assert!(
            String::from_utf8_lossy(&app_module.into_body().collect().await.unwrap().to_bytes())
                .contains("./plan-diagrams.mjs")
        );

        let diagram_module = send(
            state.clone(),
            Method::GET,
            "/plan-diagrams.mjs",
            json!({}),
            false,
        )
        .await;
        assert_eq!(diagram_module.status(), StatusCode::OK);
        assert!(
            diagram_module.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("javascript")
        );

        let renderer = send(
            state,
            Method::GET,
            "/vendor/mermaid.tiny.js",
            json!({}),
            false,
        )
        .await;
        assert_eq!(renderer.status(), StatusCode::OK);
        assert!(
            renderer.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("javascript")
        );
        let renderer_bytes = renderer.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            renderer_bytes.len(),
            PUBLIC
                .get_file("vendor/mermaid.tiny.js")
                .unwrap()
                .contents()
                .len()
        );
    }

    #[tokio::test]
    async fn loopback_api_requires_the_per_launch_token() {
        let response = send(state(), Method::GET, "/api/demo", json!({}), false).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn demo_and_analysis_are_served_by_the_rust_api() {
        let state = state();
        let demo = send(state.clone(), Method::GET, "/api/demo", json!({}), true).await;
        assert_eq!(demo.status(), StatusCode::OK);
        let demo: Value =
            serde_json::from_slice(&demo.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(demo["source"]["kind"], "synthetic");
        let selected = demo["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["kind"] == "form")
            .unwrap()["id"]
            .clone();
        let analysis = send(
            state,
            Method::POST,
            "/api/analyze",
            json!({"inventory":demo,"selectedIds":[selected],"usage":{}}),
            true,
        )
        .await;
        assert_eq!(analysis.status(), StatusCode::OK);
        let analysis: Value =
            serde_json::from_slice(&analysis.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            analysis["analysis"]["selectedIds"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty())
        );
        assert!(
            analysis["targets"]
                .as_array()
                .is_some_and(|targets| !targets.is_empty())
        );
    }

    #[tokio::test]
    async fn bundled_html_receives_only_the_local_api_token() {
        let response = send(state(), Method::GET, "/", json!({}), false).await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("name=\"access2future-token\" content=\"test-token\""));
    }

    #[tokio::test]
    async fn cancel_returns_worker_cleanup_failure_instead_of_success() {
        let state = state();
        let (cancel, _) = watch::channel(false);
        let (done, _) = watch::channel(true);
        let job = Arc::new(ImportJob {
            request_id: Uuid::new_v4().to_string(),
            snapshot: Mutex::new(ImportSnapshot {
                request_id: Uuid::new_v4().to_string(),
                state: "failed".into(),
                phase: "analysing".into(),
                completed: 0,
                total: None,
            }),
            cancel,
            done,
            result: Mutex::new(Some(Err(AccessFailure {
                code: "ACCESS_CLEANUP_FAILED",
                message: "一時ファイルを削除できませんでした。",
                status: 500,
            }))),
        });
        *state.0.active_import.lock().await = Some(job);

        let response = cancel_import(State(state), HeaderMap::new()).await;

        assert!(
            matches!(response, Err(ApiError(status, "ACCESS_CLEANUP_FAILED", _)) if status == StatusCode::INTERNAL_SERVER_ERROR)
        );
    }

    #[tokio::test]
    async fn every_cancel_waiter_observes_the_worker_result() {
        let state = state();
        let (cancel, mut cancel_rx) = watch::channel(false);
        let (done, _) = watch::channel(false);
        let job = Arc::new(ImportJob {
            request_id: Uuid::new_v4().to_string(),
            snapshot: Mutex::new(ImportSnapshot {
                request_id: Uuid::new_v4().to_string(),
                state: "running".into(),
                phase: "analysing".into(),
                completed: 0,
                total: None,
            }),
            cancel,
            done,
            result: Mutex::new(None),
        });
        *state.0.active_import.lock().await = Some(job.clone());

        let worker_job = job.clone();
        let worker = tokio::spawn(async move {
            if !*cancel_rx.borrow() {
                cancel_rx.changed().await.unwrap();
            }
            *worker_job.result.lock().await = Some(Err(AccessFailure {
                code: "ACCESS_CANCELLED",
                message: "解析を中止しました。",
                status: 499,
            }));
            worker_job.done.send_replace(true);
        });
        let first = tokio::spawn(cancel_import(State(state.clone()), HeaderMap::new()));
        let second = tokio::spawn(cancel_import(State(state), HeaderMap::new()));

        let first = tokio::time::timeout(std::time::Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        worker.await.unwrap();
        assert_eq!(first.0["cancelled"], json!(true));
        assert_eq!(second.0["cancelled"], json!(true));
    }

    #[tokio::test]
    async fn cancel_after_result_consumption_returns_without_waiting_forever() {
        let state = state();
        let (cancel, _) = watch::channel(false);
        let (done, _) = watch::channel(true);
        let job = Arc::new(ImportJob {
            request_id: Uuid::new_v4().to_string(),
            snapshot: Mutex::new(ImportSnapshot {
                request_id: Uuid::new_v4().to_string(),
                state: "completed".into(),
                phase: "finishing".into(),
                completed: 1,
                total: Some(1),
            }),
            cancel,
            done,
            result: Mutex::new(None),
        });
        *state.0.active_import.lock().await = Some(job);

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cancel_import(State(state), HeaderMap::new()),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.0["cancelled"], json!(false));
    }

    #[tokio::test]
    async fn cancel_with_a_different_import_id_does_not_stop_the_active_job() {
        let state = state();
        let request_id = Uuid::new_v4().to_string();
        let (cancel, cancel_rx) = watch::channel(false);
        let (done, _) = watch::channel(false);
        let job = Arc::new(ImportJob {
            request_id: request_id.clone(),
            snapshot: Mutex::new(ImportSnapshot {
                request_id,
                state: "running".into(),
                phase: "analysing".into(),
                completed: 2,
                total: Some(6),
            }),
            cancel,
            done,
            result: Mutex::new(None),
        });
        *state.0.active_import.lock().await = Some(job);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-import-id",
            HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap(),
        );

        let response = cancel_import(State(state), headers).await.unwrap();

        assert_eq!(response.0["cancelled"], json!(false));
        assert!(!*cancel_rx.borrow());
    }

    #[tokio::test]
    async fn cancel_reports_cleanup_failure_after_job_leaves_active_slot() {
        let state = state();
        let request_id = Uuid::new_v4().to_string();
        *state.0.last_import.lock().await = Some(ImportSnapshot {
            request_id: request_id.clone(),
            state: "failed".into(),
            phase: "finishing".into(),
            completed: 6,
            total: Some(6),
        });
        *state.0.last_import_error.lock().await = Some((
            request_id.clone(),
            AccessFailure {
                code: "ACCESS_CLEANUP_FAILED",
                message: "一時ファイルを削除できませんでした。",
                status: 500,
            },
        ));
        let mut headers = HeaderMap::new();
        headers.insert("x-import-id", HeaderValue::from_str(&request_id).unwrap());

        let response = cancel_import(State(state), headers).await;

        assert!(
            matches!(response, Err(ApiError(status, "ACCESS_CLEANUP_FAILED", _)) if status == StatusCode::INTERNAL_SERVER_ERROR)
        );
    }
}
