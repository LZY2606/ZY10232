//! 运行记录与数据的导出、清空后重新导入并复算复核。

use std::collections::BTreeMap;

use rusqlite::params;
use serde_json::{json, Value};

use crate::analysis::analyze;
use crate::db;
use crate::models::RunRequest;

/// 导出全部数据 + 全部运行（运行含配置，可被原样复算复核）。
pub fn export_all(conn: &rusqlite::Connection) -> rusqlite::Result<Value> {
    let taxa = db::list_taxa(conn)?;
    let models = db::list_age_models(conn)?;
    let samples = db::list_samples(conn)?;
    let mixed = db::list_mixed(conn)?;
    let counts = db::list_counts(conn)?;
    let runs = db::list_runs(conn)?;

    let mut age_rows: Vec<Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT sample_code,age_model_version,young,old FROM sample_ages ORDER BY sample_code,age_model_version",
        )?;
        let q = stmt.query_map([], |r| {
            Ok(json!({
                "sample_code": r.get::<_, String>(0)?,
                "age_model_version": r.get::<_, String>(1)?,
                "young": r.get::<_, f64>(2)?,
                "old": r.get::<_, f64>(3)?,
            }))
        })?;
        for r in q {
            age_rows.push(r?);
        }
    }
    let mut run_full: Vec<Value> = Vec::new();
    for r in &runs {
        let id = r["id"].as_i64().unwrap();
        if let Some(result) = db::get_run_result(conn, id)? {
            run_full.push(json!({ "id": id, "summary": r, "result": result }));
        }
    }

    Ok(json!({
        "format": "pollen-stage-export",
        "version": 1,
        "taxa": taxa,
        "age_models": models,
        "samples": samples.iter().map(|s| json!({
            "code": s.code,
            "depth_top_cm": s.depth_top_cm,
            "depth_bottom_cm": s.depth_bottom_cm,
            "label": s.label,
        })).collect::<Vec<_>>(),
        "sample_ages": age_rows,
        "mixed_layers": mixed,
        "counts": counts,
        "runs": run_full,
    }))
}

#[derive(Debug)]
pub struct ImportReport {
    pub imported_runs: usize,
    pub replayed_runs: usize,
    pub mismatches: Vec<String>,
}

fn req_from_run(run: &Value) -> Result<RunRequest, String> {
    let cfg = &run["result"]["config"];
    let groups = cfg["denominator_groups"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    });
    Ok(RunRequest {
        label: run["result"]["label"].as_str().map(String::from),
        denominator_groups: groups,
        min_sum: cfg["min_sum"].as_i64().ok_or("missing min_sum")?,
        block_size: cfg["block_size"].as_i64().ok_or("missing block_size")?,
        age_model_version: cfg["age_model_version"]
            .as_str()
            .ok_or("missing age_model_version")?
            .to_string(),
    })
}

/// 校验导出 JSON 的基本结构与引用完整性。
fn validate(data: &Value) -> Result<(), String> {
    let must = [
        "taxa",
        "age_models",
        "samples",
        "sample_ages",
        "mixed_layers",
        "counts",
        "runs",
    ];
    for k in must {
        if data.get(k).is_none() {
            return Err(format!("缺少字段 {k}"));
        }
    }
    let mut taxon_codes = BTreeMap::new();
    for t in data["taxa"].as_array().ok_or("taxa 非数组")? {
        let c = t["code"].as_str().ok_or("taxon 缺 code")?;
        if !taxon_codes.insert(c.to_string(), ()).is_none() {
            return Err(format!("分类单元重复 {c}"));
        }
    }
    let mut model_versions = BTreeMap::new();
    for m in data["age_models"].as_array().ok_or("age_models 非数组")? {
        let v = m["version"].as_str().ok_or("年代模型缺 version")?;
        model_versions.insert(v.to_string(), ());
    }
    let mut sample_codes = BTreeMap::new();
    for s in data["samples"].as_array().ok_or("samples 非数组")? {
        let c = s["code"].as_str().ok_or("样本缺 code")?;
        sample_codes.insert(c.to_string(), ());
    }
    for a in data["sample_ages"].as_array().ok_or("sample_ages 非数组")? {
        let sc = a["sample_code"].as_str().ok_or("年代缺 sample_code")?;
        let mv = a["age_model_version"].as_str().ok_or("年代缺 model")?;
        if !sample_codes.contains_key(sc) {
            return Err(format!("年代引用未知样本 {sc}"));
        }
        if !model_versions.contains_key(mv) {
            return Err(format!("年代引用未知模型 {mv}"));
        }
    }
    for c in data["counts"].as_array().ok_or("counts 非数组")? {
        let sc = c["sample_code"].as_str().ok_or("计数缺 sample_code")?;
        let tc = c["taxon_code"].as_str().ok_or("计数缺 taxon_code")?;
        if !sample_codes.contains_key(sc) {
            return Err(format!("计数引用未知样本 {sc}"));
        }
        if !taxon_codes.contains_key(tc) {
            return Err(format!("计数引用未知分类单元 {tc}"));
        }
    }
    Ok(())
}

/// 清空数据库后导入导出包，事务内写入基础数据；随后逐运行复算并与原结果比对。
pub fn import_and_verify(
    conn: &mut rusqlite::Connection,
    data: &Value,
) -> Result<ImportReport, String> {
    validate(data)?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(
        r#"
        DELETE FROM counts;
        DELETE FROM sample_ages;
        DELETE FROM mixed_layers;
        DELETE FROM runs;
        DELETE FROM samples;
        DELETE FROM age_models;
        DELETE FROM taxa;
        DELETE FROM meta;
        "#,
    )
    .map_err(|e| e.to_string())?;
    db::init_schema(&tx).map_err(|e| e.to_string())?;

    for (i, t) in data["taxa"].as_array().unwrap().iter().enumerate() {
        tx.execute(
            "INSERT INTO taxa(code,name,eco_group,default_sum,sort_order) VALUES(?1,?2,?3,?4,?5)",
            params![
                t["code"].as_str().unwrap(),
                t["name"].as_str().unwrap(),
                t["eco_group"].as_str().unwrap(),
                t["default_sum"].as_i64().unwrap_or(0),
                i as i64
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for (i, m) in data["age_models"].as_array().unwrap().iter().enumerate() {
        tx.execute(
            "INSERT INTO age_models(version,label,sort_order) VALUES(?1,?2,?3)",
            params![
                m["version"].as_str().unwrap(),
                m["label"].as_str().unwrap_or(""),
                i as i64
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for (i, s) in data["samples"].as_array().unwrap().iter().enumerate() {
        tx.execute(
            "INSERT INTO samples(code,depth_top_cm,depth_bottom_cm,label,sort_order) VALUES(?1,?2,?3,?4,?5)",
            params![
                s["code"].as_str().unwrap(),
                s["depth_top_cm"].as_f64().unwrap(),
                s["depth_bottom_cm"].as_f64().unwrap(),
                s["label"].as_str().unwrap_or(""),
                i as i64
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for a in data["sample_ages"].as_array().unwrap() {
        tx.execute(
            "INSERT INTO sample_ages(sample_code,age_model_version,young,old) VALUES(?1,?2,?3,?4)",
            params![
                a["sample_code"].as_str().unwrap(),
                a["age_model_version"].as_str().unwrap(),
                a["young"].as_f64().unwrap(),
                a["old"].as_f64().unwrap()
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for m in data["mixed_layers"].as_array().unwrap() {
        tx.execute(
            "INSERT INTO mixed_layers(depth_top_cm,depth_bottom_cm,note) VALUES(?1,?2,?3)",
            params![
                m["depth_top_cm"].as_f64().unwrap(),
                m["depth_bottom_cm"].as_f64().unwrap(),
                m["note"].as_str().unwrap_or("")
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for c in data["counts"].as_array().unwrap() {
        let count = c["count"].as_i64();
        tx.execute(
            "INSERT INTO counts(sample_code,taxon_code,count) VALUES(?1,?2,?3)",
            params![
                c["sample_code"].as_str().unwrap(),
                c["taxon_code"].as_str().unwrap(),
                count
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    // 在提交前无法跨 &mut 事务复算；先提交基础数据，再复算运行。
    tx.commit().map_err(|e| e.to_string())?;

    let runs = data["runs"].as_array().cloned().unwrap_or_default();
    let mut mismatches = Vec::new();
    let mut replayed = 0usize;

    for run in &runs {
        let created_at = run["result"]["created_at"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let label = run["result"]["label"]
            .as_str()
            .unwrap_or("分带")
            .to_string();
        let req = match req_from_run(run) {
            Ok(r) => r,
            Err(e) => {
                mismatches.push(format!("运行配置无法解析：{e}"));
                continue;
            }
        };
        let ds = db::load_dataset(conn, &req.age_model_version).map_err(|e| e.to_string())?;
        let temp_id = -1_i64;
        let recomputed = analyze(&ds, &req, temp_id, &created_at);
        let expected = &run["result"];

        // 比较确定性字段（忽略 run_id）。
        let keys = [
            "config",
            "vocab_fingerprint",
            "samples",
            "blocks",
            "boundaries",
            "transforms",
        ];
        for k in keys {
            if recomputed.get(k) != expected.get(k) {
                mismatches.push(format!(
                    "运行 {} 复算不一致：字段 {k}",
                    run["id"].as_i64().unwrap_or(-1)
                ));
            }
        }

        let config_json = serde_json::to_string(&json!({
            "denominator_groups": recomputed["config"]["denominator_groups"],
            "min_sum": req.min_sum,
            "block_size": req.block_size,
            "age_model_version": req.age_model_version,
        }))
        .map_err(|e| e.to_string())?;
        let mut stored = recomputed;
        let new_id = db::insert_run(conn, &created_at, &label, &config_json, &stored.to_string())
            .map_err(|e| e.to_string())?;
        stored["run_id"] = json!(new_id);
        // 更新结果内 run_id
        conn.execute(
            "UPDATE runs SET result_json=?1 WHERE id=?2",
            params![stored.to_string(), new_id],
        )
        .map_err(|e| e.to_string())?;
        replayed += 1;
    }

    Ok(ImportReport {
        imported_runs: runs.len(),
        replayed_runs: replayed,
        mismatches,
    })
}
