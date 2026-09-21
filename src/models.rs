use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub site: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_age_unit")]
    pub age_unit: String,
    pub taxa: Vec<Taxon>,
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<String>>,
    pub samples: Vec<RawSample>,
    pub counts: Vec<BTreeMap<String, serde_json::Value>>,
}

fn default_age_unit() -> String {
    "cal BP".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taxon {
    pub code: String,
    #[serde(default)]
    pub name_cn: String,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSample {
    pub code: String,
    pub depth_cm: f64,
    #[serde(default)]
    pub mixed: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub ages: BTreeMap<String, [f64; 2]>,
}

/// 载入并校验固定柱样：缺失(null)与零(0)严格区分。
pub fn load_fixture(json_text: &str) -> Result<Fixture, String> {
    let fx: Fixture =
        serde_json::from_str(json_text).map_err(|e| format!("fixture JSON 解析失败: {e}"))?;
    if fx.taxa.is_empty() {
        return Err("fixture 缺少分类单元".into());
    }
    if fx.samples.is_empty() {
        return Err("fixture 缺少样本".into());
    }
    let taxon_codes: std::collections::BTreeSet<&str> =
        fx.taxa.iter().map(|t| t.code.as_str()).collect();
    for (gi, members) in &fx.groups {
        for m in members {
            if !taxon_codes.contains(m.as_str()) {
                return Err(format!("固定组 {gi} 含未知分类单元 {m}"));
            }
        }
    }
    let sample_codes: std::collections::BTreeSet<&str> =
        fx.samples.iter().map(|s| s.code.as_str()).collect();
    if sample_codes.len() != fx.samples.len() {
        return Err("样本编码重复".into());
    }
    for row in &fx.counts {
        let sample = row
            .get("sample")
            .and_then(|v| v.as_str())
            .ok_or("计数行缺少 sample")?;
        if !sample_codes.contains(sample) {
            return Err(format!("计数行引用未知样本 {sample}"));
        }
        for (k, v) in row {
            if k == "sample" {
                continue;
            }
            if !taxon_codes.contains(k.as_str()) {
                return Err(format!("样本 {sample} 计数含未知分类单元 {k}"));
            }
            if let Some(n) = v.as_i64() {
                if n < 0 {
                    return Err(format!("样本 {sample} 的 {k} 计数为负"));
                }
            } else if !v.is_null() {
                return Err(format!("样本 {sample} 的 {k} 计数必须为非负整数或 null"));
            }
        }
    }
    Ok(fx)
}

/// 每个样本的计数表：None = 缺失(未观察/未记录)，Some(n) = 观察计数(可为 0)。
pub fn sample_count_map(fx: &Fixture, sample_code: &str) -> BTreeMap<String, Option<u64>> {
    let row = fx
        .counts
        .iter()
        .find(|c| c.get("sample").and_then(|v| v.as_str()) == Some(sample_code));
    fx.taxa
        .iter()
        .map(|t| {
            let val = row.and_then(|r| r.get(&t.code));
            let n: Option<u64> = match val {
                Some(serde_json::Value::Number(num)) => num.as_u64(),
                _ => None,
            };
            (t.code.clone(), n)
        })
        .collect()
}
