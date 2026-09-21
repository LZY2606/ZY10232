use crate::db::Db;
use crate::models::load_fixture;
use crate::stats::{run as run_stats, RunInput, RunResult};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub fixture_json: Arc<String>,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/static/app.js", get(app_js))
        .route("/static/style.css", get(style_css))
        .route("/api/health", get(health))
        .route("/api/fixture", get(fixture))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/:id", get(get_run).delete(delete_run))
        .route("/api/runs/:id/export", get(export_one))
        .route("/api/export", get(export_all))
        .route("/api/replay", post(replay))
        .route("/api/admin/reset", post(reset))
        .with_state(state)
}

async fn index() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(include_str!("../assets/index.html")))
        .unwrap()
}
async fn app_js() -> Response {
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )
        .body(Body::from(include_str!("../assets/app.js")))
        .unwrap()
}
async fn style_css() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .body(Body::from(include_str!("../assets/style.css")))
        .unwrap()
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

async fn health(State(st): State<AppState>) -> impl IntoResponse {
    let runs = st.db.count_runs().unwrap_or(-1);
    Json(
        json!({"ok": true, "site": load_fixture(&st.fixture_json).map(|f| f.site).unwrap_or_default(), "runs": runs}),
    )
}

async fn fixture(State(st): State<AppState>) -> Response {
    match load_fixture(&st.fixture_json) {
        Ok(fx) => match serde_json::to_value(&fx) {
            Ok(v) => Json(json!({"fixture": v, "fixture_hash": crate::stats::hash_fixture(&fx)}))
                .into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("固定柱样损坏: {e}"),
        ),
    }
}

async fn list_runs(State(st): State<AppState>) -> Response {
    match st.db.list_runs() {
        Ok(rows) => Json(json!({"runs": rows})).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn create_run(State(st): State<AppState>, Json(input): Json<RunInput>) -> Response {
    let fx = match load_fixture(&st.fixture_json) {
        Ok(f) => f,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match run_stats(&fx, input.clone()) {
        Ok(result) => match st.db.insert_run(&input, &result) {
            Ok(id) => Json(json!({"id": id, "result": result})).into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        },
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn get_run(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match st.db.get_run(id) {
        Ok(Some(rec)) => {
            let result: Value = match serde_json::from_str(&rec.result_json) {
                Ok(v) => v,
                Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            let input: Value = serde_json::from_str(&rec.input_json).unwrap_or(Value::Null);
            Json(json!({"id": id, "input": input, "result": result})).into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "运行记录不存在"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn delete_run(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match st.db.delete_run(id) {
        Ok(true) => Json(json!({"deleted": id})).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "运行记录不存在"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn export_one(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match st.db.get_run(id) {
        Ok(Some(rec)) => {
            let result: Value = serde_json::from_str(&rec.result_json).unwrap_or(Value::Null);
            let input: Value = serde_json::from_str(&rec.input_json).unwrap_or(Value::Null);
            download(
                json!({"export_kind":"pollen-succession-stage-run","export_version":1,"id":id,"input":input,"result":result}),
                format!("pollen-run-{id}.json"),
            )
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "运行记录不存在"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn export_all(State(st): State<AppState>) -> Response {
    match st.db.export_bundle() {
        Ok(text) => download_text(text, "pollen-runs-bundle.json".to_string()),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

fn download(value: Value, filename: String) -> Response {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    download_text(text, filename)
}

fn download_text(text: String, filename: String) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(text))
        .unwrap()
}

/// 重放：不直接采信导出的 result，而是用其中的 input 在当前固定柱样上重新计算，
/// 与导出结果逐字段比较；仅当完全一致且版本指纹匹配时才落库。
async fn replay(State(st): State<AppState>, Json(payload): Json<Value>) -> Response {
    let fx = match load_fixture(&st.fixture_json) {
        Ok(f) => f,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let records = extract_records(&payload);
    if records.is_empty() {
        return err(StatusCode::BAD_REQUEST, "没有可重放的运行记录");
    }
    let mut report = Vec::new();
    let mut imported = 0;
    for (idx, item) in records.iter().enumerate() {
        let input_v = item.get("input").cloned().unwrap_or(Value::Null);
        let expected_v = item.get("result").cloned().unwrap_or(Value::Null);
        let input: RunInput = match serde_json::from_value(input_v.clone()) {
            Ok(v) => v,
            Err(e) => {
                report.push(
                    json!({"index": idx, "ok": false, "reason": format!("input 解析失败: {e}")}),
                );
                continue;
            }
        };
        let recomputed: RunResult = match run_stats(&fx, input) {
            Ok(r) => r,
            Err(e) => {
                report.push(json!({"index": idx, "ok": false, "reason": format!("重算失败: {e}")}));
                continue;
            }
        };
        let actual_v = serde_json::to_value(&recomputed).unwrap_or(Value::Null);
        let expected_hash = expected_v
            .get("fixture_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let hash_ok = expected_hash == recomputed.fixture_hash;
        let body_ok = canonical(&actual_v) == canonical(&expected_v);
        if hash_ok && body_ok {
            match st
                .db
                .insert_run(&serde_json::from_value(input_v).unwrap(), &recomputed)
            {
                Ok(new_id) => {
                    imported += 1;
                    report.push(json!({"index": idx, "ok": true, "new_id": new_id,
                        "fixture_hash": recomputed.fixture_hash,
                        "vocabulary_hash": recomputed.vocabulary_hash}));
                }
                Err(e) => report.push(json!({"index": idx, "ok": false, "reason": e})),
            }
        } else {
            report.push(json!({"index": idx, "ok": false,
                "fixture_hash_match": hash_ok,
                "body_match": body_ok,
                "reason": "重算结果与导出记录不一致：固定柱样或词表版本可能已变更"}));
        }
    }
    Json(json!({"imported": imported, "checked": records.len(), "report": report})).into_response()
}

fn extract_records(payload: &Value) -> Vec<Value> {
    if let Some(runs) = payload.get("runs").and_then(|v| v.as_array()) {
        return runs.clone();
    }
    if payload.get("export_kind").is_some() && payload.get("result").is_some() {
        return vec![payload.clone()];
    }
    if payload.get("input").is_some() && payload.get("result").is_some() {
        return vec![payload.clone()];
    }
    Vec::new()
}

fn canonical(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct ResetBody {
    #[serde(default)]
    scope: Option<String>,
}

/// reset：runs=只清运行；all=清空数据库后重新导入固定柱样并复核。
async fn reset(State(st): State<AppState>, body: Option<Json<ResetBody>>) -> Response {
    let scope = body
        .and_then(|Json(b)| b.scope)
        .unwrap_or_else(|| "runs".to_string());
    match scope.as_str() {
        "runs" => match st.db.clear_runs() {
            Ok(()) => Json(json!({"reset": "runs", "runs": 0})).into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        },
        "all" => match load_fixture(&st.fixture_json) {
            Ok(fx) => match st
                .db
                .store_fixture(&fx, &st.fixture_json)
                .and_then(|_| st.db.clear_runs())
            {
                Ok(()) => Json(json!({"reset": "all", "site": fx.site,
                    "fixture_hash": crate::stats::hash_fixture(&fx), "runs": 0}))
                .into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
            },
            Err(e) => err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("固定柱样校验失败，拒绝重置: {e}"),
            ),
        },
        other => err(StatusCode::BAD_REQUEST, format!("未知 reset 范围: {other}")),
    }
}
