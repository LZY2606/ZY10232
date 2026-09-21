use crate::models::Fixture;
use crate::stats::{RunInput, RunResult};
use rusqlite::Connection;
use std::sync::Mutex;

pub struct Db {
    pub conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  run_label      TEXT NOT NULL,
  site           TEXT NOT NULL,
  transform      TEXT NOT NULL,
  age_model      TEXT NOT NULL,
  min_total      INTEGER NOT NULL,
  block_size     INTEGER NOT NULL,
  denominator    TEXT NOT NULL,
  fixture_hash   TEXT NOT NULL,
  vocabulary_hash TEXT NOT NULL,
  input_json     TEXT NOT NULL,
  result_json    TEXT NOT NULL,
  created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

impl Db {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("打开数据库失败 {path}: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("建表失败: {e}"))?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    pub fn store_fixture(&self, fx: &Fixture, raw: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('fixture_json', ?1)",
            [raw],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('site', ?1)",
            [fx.site.clone()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn fixture_json(&self) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row("SELECT value FROM meta WHERE key='fixture_json'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    }

    pub fn has_fixture(&self) -> bool {
        self.fixture_json().is_some()
    }

    pub fn insert_run(&self, input: &RunInput, result: &RunResult) -> Result<i64, String> {
        let input_json = serde_json::to_string(input).map_err(|e| e.to_string())?;
        let result_json = serde_json::to_string(result).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO runs(run_label, site, transform, age_model, min_total, block_size,
                 denominator, fixture_hash, vocabulary_hash, input_json, result_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                result.run_label,
                result.site,
                result.transform.as_str(),
                result.age_model,
                result.min_total as i64,
                result.block_size as i64,
                serde_json::to_string(&result.denominator_taxa).unwrap_or_default(),
                result.fixture_hash,
                result.vocabulary_hash,
                input_json,
                result_json,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_runs(&self) -> Result<Vec<RunSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, run_label, transform, age_model, min_total, block_size,
                        denominator, fixture_hash, vocabulary_hash, created_at
                 FROM runs ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RunSummary {
                    id: r.get(0)?,
                    run_label: r.get(1)?,
                    transform: r.get(2)?,
                    age_model: r.get(3)?,
                    min_total: r.get(4)?,
                    block_size: r.get(5)?,
                    denominator: r.get(6)?,
                    fixture_hash: r.get(7)?,
                    vocabulary_hash: r.get(8)?,
                    created_at: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn get_run(&self, id: i64) -> Result<Option<StoredRun>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut s = conn
            .prepare("SELECT input_json, result_json FROM runs WHERE id=?1")
            .map_err(|e| e.to_string())?;
        let mut rows = s.query([id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let input_json: String = row.get(0).map_err(|e| e.to_string())?;
            let result_json: String = row.get(1).map_err(|e| e.to_string())?;
            Ok(Some(StoredRun {
                input_json,
                result_json,
            }))
        } else {
            Ok(None)
        }
    }

    /// 导出全部运行记录（含固定柱样版本指纹，便于清空后复核）。
    pub fn export_bundle(&self) -> Result<String, String> {
        let runs = self.list_runs()?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT input_json, result_json FROM runs ORDER BY id")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let mut records: Vec<serde_json::Value> = Vec::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let input_json: String = r.get(0).map_err(|e| e.to_string())?;
            let result_json: String = r.get(1).map_err(|e| e.to_string())?;
            records.push(serde_json::json!({
                "input": serde_json::from_str::<serde_json::Value>(&input_json).unwrap_or(serde_json::Value::Null),
                "result": serde_json::from_str::<serde_json::Value>(&result_json).unwrap_or(serde_json::Value::Null),
            }));
        }
        let bundle = serde_json::json!({
            "export_kind": "pollen-succession-stage-runs",
            "export_version": 1,
            "run_count": runs.len(),
            "runs": records,
        });
        serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())
    }

    pub fn count_runs(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    pub fn delete_run(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let n = conn
            .execute("DELETE FROM runs WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn clear_runs(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch("DELETE FROM runs; DELETE FROM sqlite_sequence WHERE name='runs';")
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct RunSummary {
    pub id: i64,
    pub run_label: String,
    pub transform: String,
    pub age_model: String,
    pub min_total: i64,
    pub block_size: i64,
    pub denominator: String,
    pub fixture_hash: String,
    pub vocabulary_hash: String,
    pub created_at: String,
}

pub struct StoredRun {
    pub input_json: String,
    pub result_json: String,
}
