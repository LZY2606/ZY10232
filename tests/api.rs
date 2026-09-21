use axum::body::Body;
use axum::body::Bytes;
use axum::Router;
use http_body_util::BodyExt;
use pollen_stage::build_state_with_db;
use pollen_stage::db::Db;
use std::sync::Arc;
use tower::util::ServiceExt;

use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_db() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!(
        "pollen-stage-test-{}-{n}-{seq}.sqlite3",
        std::process::id()
    ));
    p.to_string_lossy().to_string()
}

struct Ctx {
    app: Router,
    path: String,
}

fn ctx() -> Ctx {
    let path = temp_db();
    let db = Arc::new(Db::open(&path).unwrap());
    pollen_stage::seed_if_needed(&db).unwrap();
    Ctx {
        app: pollen_stage::web::app(build_state_with_db(db)),
        path,
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(format!("{}-wal", self.path));
        let _ = std::fs::remove_file(format!("{}-shm", self.path));
    }
}

async fn call(app: &Router, req: http::Request<Body>) -> (http::StatusCode, String) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes: Bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn get(path: &str) -> http::Request<Body> {
    http::Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}
fn post_json(path: &str, json: &str) -> http::Request<Body> {
    http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(json.to_string()))
        .unwrap()
}
fn del(path: &str) -> http::Request<Body> {
    http::Request::builder()
        .method("DELETE")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn index_shows_title_and_health() {
    let c = ctx();
    let (s, body) = call(&c.app, get("/")).await;
    assert_eq!(s, 200);
    assert!(body.contains("孢粉演替台"), "首页必须含标题");
    let (s, body) = call(&c.app, get("/api/health")).await;
    assert_eq!(s, 200);
    assert!(body.contains("\"ok\":true"));
    let (s, body) = call(&c.app, get("/api/fixture")).await;
    assert_eq!(s, 200);
    assert!(body.contains("fixture_hash"));
}

#[tokio::test]
async fn create_run_zero_total_no_allzero_percent() {
    let c = ctx();
    let input = serde_json::json!({
        "group": "all", "min_total": 0, "block_size": 1,
        "age_model": "am_2024", "transform": "proportion"
    })
    .to_string();
    let (s, body) = call(&c.app, post_json("/api/runs", &input)).await;
    assert_eq!(s, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["id"].is_number());
    let samples = v["result"]["samples"].as_array().unwrap();
    let s4 = samples.iter().find(|s| s["code"] == "S4").unwrap();
    assert_eq!(s4["observed_total"], 0);
    assert!(s4["percents"].is_null());
    assert!(s4["proportions"].is_null());
    assert!(s4["excluded_reason"].is_string());
}

#[tokio::test]
async fn denominator_switch_changes_support_and_vocabulary() {
    let c = ctx();
    let run1 = serde_json::json!({
        "group": "all", "min_total": 30, "block_size": 1,
        "age_model": "am_2024", "transform": "proportion"
    })
    .to_string();
    let run2 = serde_json::json!({
        "group": "pollen_only", "min_total": 30, "block_size": 1,
        "age_model": "am_2024", "transform": "proportion"
    })
    .to_string();
    let (_, b1) = call(&c.app, post_json("/api/runs", &run1)).await;
    let (_, b2) = call(&c.app, post_json("/api/runs", &run2)).await;
    let v1: serde_json::Value = serde_json::from_str(&b1).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&b2).unwrap();
    assert_ne!(
        v1["result"]["vocabulary_hash"],
        v2["result"]["vocabulary_hash"]
    );
    fn find_support(v: &serde_json::Value) -> f64 {
        v["result"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["between"][0] == "S8" && b["between"][1] == "S9")
            .unwrap()["support_proportion"]
            .as_f64()
            .unwrap()
    }
    assert_ne!(find_support(&v1), find_support(&v2));
}

#[tokio::test]
async fn disjoint_age_keeps_multi_peak() {
    let c = ctx();
    let input = serde_json::json!({
        "group": "all", "min_total": 30, "block_size": 1,
        "age_model": "am_2024", "transform": "angular"
    })
    .to_string();
    let (s, body) = call(&c.app, post_json("/api/runs", &input)).await;
    assert_eq!(s, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let b = v["result"]["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["between"][0] == "S6" && b["between"][1] == "S8")
        .unwrap();
    assert_eq!(b["skipped_samples"][0], "S7");
    assert_eq!(b["multi_peak"], true);
    let peaks = b["age_peaks"].as_array().unwrap();
    assert!(peaks.len() >= 2, "年代不相交必须保留多峰");
    for p in peaks {
        assert!(p[0].as_f64().unwrap() <= p[1].as_f64().unwrap());
    }
    assert!(b["peak_note"].is_string());
}

#[tokio::test]
async fn replay_roundtrip_and_tamper_rejection() {
    let c = ctx();
    let input = serde_json::json!({
        "group": "trees", "min_total": 30, "block_size": 1,
        "age_model": "am_2019", "transform": "counts"
    })
    .to_string();
    let (cs, body) = call(&c.app, post_json("/api/runs", &input)).await;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = created["id"].as_i64().unwrap();

    let (es, one) = call(&c.app, get(&format!("/api/runs/{id}/export"))).await;
    assert_eq!(es, 200, "导出: {one}");
    let rec: serde_json::Value = serde_json::from_str(&one).unwrap();

    // 1) 被篡改的导出记录必须被拒绝（重算结果逐字段比对）。
    let mut tampered_rec = rec.clone();
    tampered_rec["result"]["boundaries"][0]["support"] = 0.999999.into();
    let tampered = serde_json::json!({"runs": [tampered_rec]}).to_string();
    let (s, rep) = call(&c.app, post_json("/api/replay", &tampered)).await;
    assert_eq!(s, 200);
    let rv: serde_json::Value = serde_json::from_str(&rep).unwrap();
    assert_eq!(rv["imported"], 0, "被篡改记录必须拒绝");
    assert_eq!(rv["report"][0]["ok"], false);
    assert_eq!(rv["report"][0]["body_match"], false);

    // 2) 清空数据库（含运行）后，用导出包原样重放应重新计算复核并入库。
    let (s, _) = call(&c.app, post_json("/api/admin/reset", r#"{"scope":"all"}"#)).await;
    assert_eq!(s, 200);
    let (s, rep2) = call(
        &c.app,
        post_json(
            "/api/replay",
            &serde_json::json!({"runs":[rec]}).to_string(),
        ),
    )
    .await;
    assert_eq!(s, 200);
    let rv2: serde_json::Value = serde_json::from_str(&rep2).unwrap();
    assert_eq!(rv2["imported"], 1, "原样重放应复核通过");
    let (_, list) = call(&c.app, get("/api/runs")).await;
    let lv: serde_json::Value = serde_json::from_str(&list).unwrap();
    assert_eq!(lv["runs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn reset_all_reseeds_and_lists_runs() {
    let c = ctx();
    let input = r#"{"group":"herbs","min_total":0,"block_size":1,"age_model":"am_2019","transform":"proportion"}"#;
    let (s, _) = call(&c.app, post_json("/api/runs", input)).await;
    assert_eq!(s, 200);
    let (_, body) = call(&c.app, get("/api/runs")).await;
    assert!(body.contains("pollen_stage") || body.contains("LAKE-A") || body.contains("run_label"));
    let (s, body) = call(&c.app, post_json("/api/admin/reset", r#"{"scope":"all"}"#)).await;
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["runs"], 0);
    assert_eq!(v["site"], "LAKE-A");
    let (_, body) = call(&c.app, get("/api/runs")).await;
    assert!(body.contains("[]"));
    // 固定柱样仍在
    let (s, _) = call(&c.app, get("/api/fixture")).await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn invalid_input_returns_400_and_delete_404() {
    let c = ctx();
    let (s, body) = call(
        &c.app,
        post_json("/api/runs", r#"{"denominator_taxa":["Ghost"]}"#),
    )
    .await;
    assert_eq!(s, 400);
    assert!(body.contains("未知分类单元"));
    let (s, _) = call(&c.app, del("/api/runs/9999")).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn export_bundle_shape() {
    let c = ctx();
    let input = r#"{"group":"all","min_total":30,"block_size":2,"age_model":"am_2019","transform":"angular"}"#;
    let (s, _) = call(&c.app, post_json("/api/runs", input)).await;
    assert_eq!(s, 200);
    let (s, body) = call(&c.app, get("/api/export")).await;
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["export_kind"], "pollen-succession-stage-runs");
    assert!(v["run_count"].as_i64().unwrap() >= 1);
    assert!(v["runs"][0]["result"]["vocabulary_hash"].is_string());
}
