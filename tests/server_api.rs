//! HTTP 端到端测试：初始化、分带、分母切换、多峰、导出→清空→导入复核、重置。

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::body::Bytes;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use pollen_stage::analysis::vocab_fingerprint;
use pollen_stage::{db, fixture, server};
use serde_json::{json, Value};
use tower::ServiceExt;

fn temp_db() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "pstage-it-{}-{}-{}.sqlite3",
        std::process::id(),
        n,
        seq
    ));
    p.to_string_lossy().to_string()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Bytes {
    resp.into_body().collect().await.unwrap().to_bytes()
}

fn app_with(path: &str, seed: bool) -> Router {
    let mut conn = db::open(path).unwrap();
    db::init_schema(&conn).unwrap();
    if seed {
        fixture::reset(&mut conn).unwrap();
        server::seed_demo_runs(&mut conn).unwrap();
    }
    let state = server::AppState {
        db_path: path.to_string(),
        conn: Arc::new(Mutex::new(conn)),
    };
    server::router(state)
}

async fn post_json(app: &Router, uri: &str, body: Value) -> (u16, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, body_json(resp).await)
}

#[tokio::test]
async fn index_shows_title() {
    let app = app_with(&temp_db(), true);
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let html = body_bytes(resp).await;
    assert!(String::from_utf8_lossy(&html).contains("孢粉演替台"));
}

#[tokio::test]
async fn fixture_has_zero_missing_mixed_and_two_models() {
    let path = temp_db();
    let app = app_with(&path, true);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dataset")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let d = body_json(resp).await;
    let models: Vec<String> = d["age_models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["version"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(models, vec!["age-v1-linear", "age-v2-bacon"]);
    assert!(!d["mixed_layers"].as_array().unwrap().is_empty());

    // S05 全为结构零；S02 的 TYPHA 缺失。
    let counts = d["counts"].as_array().unwrap();
    let s05: Vec<&Value> = counts
        .iter()
        .filter(|c| c["sample_code"] == "S05")
        .collect();
    assert!(s05.iter().all(|c| c["count"].as_i64() == Some(0)));
    let s02_typha = counts
        .iter()
        .find(|c| c["sample_code"] == "S02" && c["taxon_code"] == "TYPHA")
        .unwrap();
    assert!(s02_typha["count"].is_null(), "缺失计数必须是 SQL NULL");
}

#[tokio::test]
async fn run_excludes_zero_and_low_samples_and_keeps_age_multimodal_v1() {
    let app = app_with(&temp_db(), true);
    let (st, v) = post_json(
        &app,
        "/api/runs",
        json!({
            "label":"it",
            "denominator_groups":["乔木","陆生草本"],
            "min_sum":50,"block_size":1,"age_model_version":"age-v1-linear"
        }),
    )
    .await;
    assert_eq!(st, 201);
    let r = &v["result"];
    let by = |c: &str| {
        r["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["code"] == c)
            .unwrap()
    };
    assert_eq!(by("S05")["exclude_reason"], json!("zero_total"));
    assert!(by("S05")["taxa"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["prop"].is_null()));
    assert!(by("S05")["taxa"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["count"] == json!(0)));
    assert_eq!(by("S04")["included"], json!(false));
    assert_eq!(by("S04")["exclude_reason"], json!("low_total(<50)"));

    // 关键林线边界 S03/S06：v1 不相交 → 多峰
    let main = r["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["upper_samples"][0] == json!("S03") && b["lower_samples"][0] == json!("S06"))
        .unwrap();
    assert_eq!(main["multi_modal"], json!(true));
    assert_eq!(main["age_peaks"].as_array().unwrap().len(), 2);
    assert!(main["gap"].as_bool().unwrap());
    // 绝不能出现单一平均年份
    let joined = main["age_peaks"].to_string();
    assert!(!joined.contains("2900"));
}

#[tokio::test]
async fn v2_merges_disjoint_boundary_into_single_peak() {
    let app = app_with(&temp_db(), true);
    let (_, v) = post_json(
        &app,
        "/api/runs",
        json!({
            "label":"it2",
            "denominator_groups":["乔木","陆生草本"],
            "min_sum":50,"block_size":1,"age_model_version":"age-v2-bacon"
        }),
    )
    .await;
    let main = v["result"]["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["upper_samples"][0] == json!("S03") && b["lower_samples"][0] == json!("S06"))
        .unwrap();
    assert_eq!(main["multi_modal"], json!(false));
    assert_eq!(main["age_peaks"].as_array().unwrap().len(), 1);
    // v2：S03=820-1020 与 S06=1000-1400 在 1000-1020 重叠 → 合并为单一交集
    assert_eq!(main["age_peaks"][0]["young"], json!(1000.0));
    assert_eq!(main["age_peaks"][0]["old"], json!(1020.0));
}

#[tokio::test]
async fn changing_denominator_changes_boundary_support() {
    let app = app_with(&temp_db(), true);
    let run = |groups: Value| {
        let app = app.clone();
        async move {
            let (_, v) = post_json(
                &app,
                "/api/runs",
                json!({
                    "min_sum":50,"block_size":1,"age_model_version":"age-v1-linear",
                    "denominator_groups":groups
                }),
            )
            .await;
            v["result"]["boundaries"][0]["hellinger"].as_f64().unwrap()
        }
    };
    let h_default = run(json!(["乔木", "陆生草本"])).await;
    // 加入水生、蕨类：分母被稀释，主边界支持度应发生变化
    let h_wide = run(json!(["乔木", "陆生草本", "水生", "蕨类"])).await;
    assert_ne!(h_default, h_wide, "验收：改变分母组须改变边界支持");
}

#[tokio::test]
async fn export_reset_import_roundtrip_verifies() {
    let path = temp_db();
    let app = app_with(&path, true);

    // 额外新建一个运行
    post_json(
        &app,
        "/api/runs",
        json!({"denominator_groups":["乔木"],"min_sum":100,"block_size":2,
               "age_model_version":"age-v1-linear","label":"复核用"}),
    )
    .await;

    // 导出
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let export = body_json(resp).await;
    assert_eq!(export["format"], json!("pollen-stage-export"));
    let n_runs = export["runs"].as_array().unwrap().len();
    assert!(n_runs >= 3);

    // 清空后导入并复核
    let (st, rep) = post_json(&app, "/api/import", export).await;
    assert_eq!(st, 200);
    assert_eq!(rep["verified"], json!(true), "复算应与导出结果一致: {rep}");
    assert_eq!(rep["mismatches"].as_array().unwrap().len(), 0);
    assert_eq!(rep["replayed_runs"].as_i64().unwrap(), n_runs as i64);

    // 重新导入后数据仍可查询
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dataset")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let d = body_json(resp).await;
    assert_eq!(d["taxa"].as_array().unwrap().len(), 10);
    let runs_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let runs = body_json(runs_resp).await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), n_runs);
}

#[tokio::test]
async fn reset_endpoint_reseeds_and_vocab_is_stable() {
    let app = app_with(&temp_db(), true);
    let (st, rep) = post_json(&app, "/api/reset", json!({})).await;
    assert_eq!(st, 200);
    assert_eq!(rep["status"], json!("reset"));
    assert_eq!(rep["fixture"], json!(db::FIXTURE_VERSION));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/taxa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tx = body_json(resp).await;
    assert!(tx["vocab_fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("vocab-v1-"));
    // 指纹确定性：同词表两次一致
    let taxa_v: Vec<pollen_stage::models::Taxon> =
        serde_json::from_value(tx["taxa"].clone()).unwrap();
    assert_eq!(
        vocab_fingerprint(&taxa_v),
        tx["vocab_fingerprint"].as_str().unwrap()
    );
}

#[tokio::test]
async fn invalid_age_model_rejected() {
    let app = app_with(&temp_db(), true);
    let (st, err) = post_json(
        &app,
        "/api/runs",
        json!({"min_sum":0,"block_size":1,"age_model_version":"nope"}),
    )
    .await;
    assert_eq!(st, 400);
    assert!(err["error"].as_str().unwrap().contains("年代模型"));
}
