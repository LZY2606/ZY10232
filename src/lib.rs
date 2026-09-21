pub mod db;
pub mod models;
pub mod stats;
pub mod web;

pub const FIXTURE_JSON: &str = include_str!("../fixtures/fixture.json");

/// 测试/启动共用：内存或临时库上装配 AppState。
pub fn build_state_with_db(db: std::sync::Arc<db::Db>) -> web::AppState {
    web::AppState {
        db,
        fixture_json: std::sync::Arc::new(FIXTURE_JSON.to_string()),
    }
}

pub fn seed_if_needed(db: &db::Db) -> Result<String, String> {
    let fx = models::load_fixture(FIXTURE_JSON)?;
    let hash = stats::hash_fixture(&fx);
    if !db.has_fixture() {
        db.store_fixture(&fx, FIXTURE_JSON)?;
    } else if db.fixture_json().as_deref() != Some(FIXTURE_JSON) {
        // 已存在但版本不一致：覆盖为随二进制发布的固定柱样，并清空旧运行。
        db.store_fixture(&fx, FIXTURE_JSON)?;
        db.clear_runs()?;
    }
    Ok(hash)
}
