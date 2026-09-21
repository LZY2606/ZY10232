//! Axum 本地服务：静态页面 + JSON API。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::analysis::analyze;
use crate::db;
use crate::fixture;
use crate::io_export;
use crate::models::RunRequest;

#[derive(Clone)]
pub struct AppState {
    pub db_path: String,
    pub conn: std::sync::Arc<Mutex<Connection>>,
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

struct AppError(StatusCode, String);
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/api/taxa", get(get_taxa))
        .route("/api/age-models", get(get_age_models))
        .route("/api/dataset", get(get_dataset))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/:id", get(get_run))
        .route("/api/export", get(export))
        .route("/api/import", post(import))
        .route("/api/reset", post(reset))
        .with_state(state)
}

async fn index() -> Response {
    let html = include_str!("../static/index.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

async fn app_js() -> Response {
    let js = include_str!("../static/app.js");
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        js,
    )
        .into_response()
}

async fn get_taxa(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    let taxa = db::list_taxa(&conn)?;
    let groups = crate::models::ECO_GROUPS;
    Ok(Json(json!({
        "taxa": taxa,
        "eco_groups": groups,
        "vocab_fingerprint": crate::analysis::vocab_fingerprint(&taxa),
    })))
}

async fn get_age_models(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    Ok(Json(json!({ "age_models": db::list_age_models(&conn)? })))
}

async fn get_dataset(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    let export = io_export::export_all(&conn)?;
    Ok(Json(json!({
        "taxa": export["taxa"],
        "age_models": export["age_models"],
        "samples": export["samples"],
        "sample_ages": export["sample_ages"],
        "mixed_layers": export["mixed_layers"],
        "counts": export["counts"],
    })))
}

async fn list_runs(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    Ok(Json(json!({ "runs": db::list_runs(&conn)? })))
}

#[derive(Debug, Deserialize)]
struct CreateRunBody {
    label: Option<String>,
    denominator_groups: Option<Vec<String>>,
    min_sum: Option<i64>,
    block_size: Option<i64>,
    age_model_version: String,
}

async fn create_run(
    State(st): State<AppState>,
    Json(body): Json<CreateRunBody>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let req = RunRequest {
        label: body.label,
        denominator_groups: body.denominator_groups,
        min_sum: body.min_sum.unwrap_or(50),
        block_size: body.block_size.unwrap_or(1).max(1),
        age_model_version: body.age_model_version.clone(),
    };
    if req.min_sum < 0 {
        return Err(AppError(StatusCode::BAD_REQUEST, "min_sum 不能为负".into()));
    }
    let conn = st.conn.lock().unwrap();
    let models = db::list_age_models(&conn)?;
    if !models.iter().any(|m| m.version == req.age_model_version) {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!("未知年代模型版本 {}", req.age_model_version),
        ));
    }
    if let Some(groups) = &req.denominator_groups {
        for g in groups {
            if !crate::models::ECO_GROUPS.contains(&g.as_str()) {
                return Err(AppError(StatusCode::BAD_REQUEST, format!("未知生态组 {g}")));
            }
        }
    }

    let created = now_iso();
    let label = req.normalized_label();
    let ds = db::load_dataset(&conn, &req.age_model_version)?;
    let temp = analyze(&ds, &req, -1, &created);
    let config_json = serde_json::to_string(&temp["config"])
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let result_json = temp.to_string();
    let id = db::insert_run(&conn, &created, &label, &config_json, &result_json)?;
    let mut stored = temp;
    stored["run_id"] = json!(id);
    conn.execute(
        "UPDATE runs SET result_json=?1 WHERE id=?2",
        rusqlite::params![stored.to_string(), id],
    )?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "result": stored })),
    ))
}

async fn get_run(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    match db::get_run_result(&conn, id)? {
        Some(v) => Ok(Json(v)),
        None => Err(AppError(StatusCode::NOT_FOUND, format!("运行 {id} 不存在"))),
    }
}

async fn export(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let conn = st.conn.lock().unwrap();
    Ok(Json(io_export::export_all(&conn)?))
}

async fn import(
    State(st): State<AppState>,
    Json(data): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let mut conn = st.conn.lock().unwrap();
    match io_export::import_and_verify(&mut conn, &data) {
        Ok(rep) => Ok((
            StatusCode::OK,
            Json(json!({
                "imported_runs": rep.imported_runs,
                "replayed_runs": rep.replayed_runs,
                "mismatches": rep.mismatches,
                "verified": rep.mismatches.is_empty(),
            })),
        )),
        Err(e) => Err(AppError(StatusCode::BAD_REQUEST, e)),
    }
}

async fn reset(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut conn = st.conn.lock().unwrap();
    fixture::reset(&mut conn)?;
    let seeded = seed_demo_runs(&mut conn)?;
    Ok(Json(json!({
        "status": "reset",
        "fixture": db::FIXTURE_VERSION,
        "seeded_demo_runs": seeded,
    })))
}

/// 初始/重置后默认演示运行：v1 多峰边界、v2 合并边界。
pub fn seed_demo_runs(conn: &mut Connection) -> rusqlite::Result<Vec<i64>> {
    let demos = [
        (
            "演示：v1 线性 · 默认分母 · 多样本组合(k=2)",
            50,
            2,
            "age-v1-linear",
        ),
        (
            "演示：v2 Bacon · 默认分母 · 逐样本(k=1)",
            50,
            1,
            "age-v2-bacon",
        ),
    ];
    let mut ids = Vec::new();
    for (label, min_sum, block_size, version) in demos {
        let req = RunRequest {
            label: Some(label.to_string()),
            denominator_groups: None,
            min_sum,
            block_size,
            age_model_version: version.to_string(),
        };
        let created = "seed:2026-09-22T00:00:00Z".to_string();
        let ds = db::load_dataset(conn, version)?;
        let temp = analyze(&ds, &req, -1, &created);
        let config_json = temp["config"].to_string();
        let id = db::insert_run(conn, &created, label, &config_json, &temp.to_string())?;
        let mut stored = temp;
        stored["run_id"] = json!(id);
        conn.execute(
            "UPDATE runs SET result_json=?1 WHERE id=?2",
            rusqlite::params![stored.to_string(), id],
        )?;
        ids.push(id);
    }
    Ok(ids)
}
