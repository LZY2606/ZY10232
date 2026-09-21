//! 数据模型：分类单元、年代模型、样本/计数、运行配置。

use serde::{Deserialize, Serialize};

/// 生态组（固定词表）。顺序也是页面展示顺序。
pub const ECO_GROUPS: &[&str] = &["乔木", "灌木", "陆生草本", "水生", "蕨类"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taxon {
    pub code: String,
    pub name: String,
    pub eco_group: String,
    pub default_sum: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgeModel {
    pub version: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub code: String,
    pub depth_top_cm: f64,
    pub depth_bottom_cm: f64,
    pub label: String,
}

/// 单个样本在某年代模型下的年代区间（cal a BP，老 = 数值大）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AgeRange {
    pub young: f64,
    pub old: f64,
}

impl AgeRange {
    pub fn new(young: f64, old: f64) -> Self {
        Self { young, old }
    }
}

/// 混层（再搬运/生物扰动）深度范围，cm，闭区间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixedLayer {
    pub depth_top_cm: f64,
    pub depth_bottom_cm: f64,
    pub note: String,
}

impl MixedLayer {
    pub fn contains(&self, depth: f64) -> bool {
        depth >= self.depth_top_cm && depth <= self.depth_bottom_cm
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountRow {
    pub sample_code: String,
    pub taxon_code: String,
    /// None 表示缺失计数（与结构零 Some(0) 严格区分）。
    pub count: Option<i64>,
}

/// 一次分带的请求配置。每次分带固定词表版本与年代模型版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub label: Option<String>,
    /// 进入分母（孢粉总和）的生态组。None 表示使用分类单元的 default_sum 词表默认。
    pub denominator_groups: Option<Vec<String>>,
    /// 排除低计数样本的最小分母总和阈值；零总计数样本恒被排除。
    pub min_sum: i64,
    /// 相邻样本组合大小（block size, k），k 个相邻样本聚合为一个区块。
    pub block_size: i64,
    /// 固定使用的年代模型版本。
    pub age_model_version: String,
}

impl RunRequest {
    pub fn normalized_label(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| format!("分带 min_sum={} k={}", self.min_sum, self.block_size))
    }
}
