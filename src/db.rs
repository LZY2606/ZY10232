//! SQLite 持久化：schema、查询、运行记录、导出/导入/重置。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde_json::Value;

use crate::analysis::{Dataset, SampleAges, SampleCounts};
use crate::models::{AgeModel, AgeRange, CountRow, MixedLayer, Taxon};

pub const SCHEMA_VERSION: i64 = 1;
pub const FIXTURE_VERSION: &str = "fixture-v1";

pub fn open(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", 1)?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS taxa (
            code TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            eco_group TEXT NOT NULL,
            default_sum INTEGER NOT NULL,
            sort_order INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS age_models (
            version TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS samples (
            code TEXT PRIMARY KEY,
            depth_top_cm REAL NOT NULL,
            depth_bottom_cm REAL NOT NULL,
            label TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS mixed_layers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            depth_top_cm REAL NOT NULL,
            depth_bottom_cm REAL NOT NULL,
            note TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sample_ages (
            sample_code TEXT NOT NULL REFERENCES samples(code),
            age_model_version TEXT NOT NULL REFERENCES age_models(version),
            young REAL NOT NULL,
            old REAL NOT NULL,
            PRIMARY KEY (sample_code, age_model_version)
        );
        CREATE TABLE IF NOT EXISTS counts (
            sample_code TEXT NOT NULL REFERENCES samples(code),
            taxon_code TEXT NOT NULL REFERENCES taxa(code),
            count INTEGER,
            PRIMARY KEY (sample_code, taxon_code)
        );
        CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            label TEXT NOT NULL,
            config_json TEXT NOT NULL,
            result_json TEXT NOT NULL
        );
        "#,
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO meta(key,value) VALUES('schema_version',?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

// ---------- 读取 ----------

pub fn list_taxa(conn: &Connection) -> rusqlite::Result<Vec<Taxon>> {
    let mut stmt =
        conn.prepare("SELECT code,name,eco_group,default_sum FROM taxa ORDER BY sort_order, code")?;
    let rows = stmt.query_map([], |r| {
        Ok(Taxon {
            code: r.get(0)?,
            name: r.get(1)?,
            eco_group: r.get(2)?,
            default_sum: r.get(3)?,
        })
    })?;
    rows.collect()
}

pub fn list_age_models(conn: &Connection) -> rusqlite::Result<Vec<AgeModel>> {
    let mut stmt =
        conn.prepare("SELECT version,label FROM age_models ORDER BY sort_order, version")?;
    let rows = stmt.query_map([], |r| {
        Ok(AgeModel {
            version: r.get(0)?,
            label: r.get(1)?,
        })
    })?;
    rows.collect()
}

pub fn list_samples(conn: &Connection) -> rusqlite::Result<Vec<SampleAges>> {
    // 与具体年代模型无关的样本列表（年龄在 load_dataset 时按版本填）
    let mut stmt = conn.prepare(
        "SELECT code,depth_top_cm,depth_bottom_cm,label FROM samples ORDER BY sort_order",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let rows: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|(code, t, b, label)| SampleAges {
            code,
            depth_top_cm: t,
            depth_bottom_cm: b,
            label,
            age: AgeRange::new(0.0, 0.0),
        })
        .collect())
}

pub fn list_mixed(conn: &Connection) -> rusqlite::Result<Vec<MixedLayer>> {
    let mut stmt =
        conn.prepare("SELECT depth_top_cm,depth_bottom_cm,note FROM mixed_layers ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(MixedLayer {
            depth_top_cm: r.get(0)?,
            depth_bottom_cm: r.get(1)?,
            note: r.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn list_counts(conn: &Connection) -> rusqlite::Result<Vec<CountRow>> {
    let mut stmt = conn.prepare("SELECT sample_code,taxon_code,count FROM counts")?;
    let rows = stmt.query_map([], |r| {
        Ok(CountRow {
            sample_code: r.get(0)?,
            taxon_code: r.get(1)?,
            count: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// 按固定年代模型版本装载分析数据集（拥有所有权，缺失计数行即视为缺失）。
pub fn load_dataset(conn: &Connection, age_version: &str) -> rusqlite::Result<Dataset> {
    let taxa = list_taxa(conn)?;
    let mut samples = list_samples(conn)?;
    {
        let mut stmt = conn
            .prepare("SELECT sample_code,young,old FROM sample_ages WHERE age_model_version=?1")?;
        let map: BTreeMap<String, (f64, f64)> = stmt
            .query_map([age_version], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, f64>(1)?, r.get::<_, f64>(2)?),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for s in samples.iter_mut() {
            if let Some((y, o)) = map.get(&s.code) {
                s.age = AgeRange::new(*y, *o);
            }
        }
    }
    let counts_rows = list_counts(conn)?;
    let mut by_sample: BTreeMap<String, SampleCounts> = BTreeMap::new();
    for r in counts_rows {
        let e = by_sample
            .entry(r.sample_code.clone())
            .or_insert_with(|| SampleCounts {
                code: r.sample_code.clone(),
                counts: BTreeMap::new(),
            });
        e.counts.insert(r.taxon_code.clone(), r.count);
    }
    let rows = by_sample.into_values().collect();
    let mixed_layers = list_mixed(conn)?;
    Ok(Dataset {
        taxa,
        samples,
        rows,
        mixed_layers,
    })
}

// ---------- 运行记录 ----------

pub fn insert_run(
    conn: &Connection,
    created_at: &str,
    label: &str,
    config_json: &str,
    result_json: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO runs(created_at,label,config_json,result_json) VALUES(?1,?2,?3,?4)",
        rusqlite::params![created_at, label, config_json, result_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_runs(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT id,created_at,label,config_json FROM runs ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        let config: String = r.get(3)?;
        let config_v: Value = serde_json::from_str(&config).unwrap_or(Value::Null);
        Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?,
            "created_at": r.get::<_, String>(1)?,
            "label": r.get::<_, String>(2)?,
            "config": config_v,
        }))
    })?;
    rows.collect()
}

pub fn get_run_result(conn: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT result_json FROM runs WHERE id=?1")?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(r) => {
            let s: String = r.get(0)?;
            Ok(Some(serde_json::from_str(&s).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(e))
            })?))
        }
        None => Ok(None),
    }
}

pub fn is_empty(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM taxa", [], |r| r.get(0))?;
    Ok(n == 0)
}
