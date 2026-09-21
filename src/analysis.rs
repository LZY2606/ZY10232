//! 统计核心：数量 / 相对比例 / Hellinger 方差稳定变换，相邻样本组合边界候选，
//! 组间差异、方向（含混层不精确边界）与年代多峰合并。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::models::{AgeRange, MixedLayer, RunRequest, Taxon};

#[derive(Debug, Clone)]
pub struct SampleAges {
    pub code: String,
    pub depth_top_cm: f64,
    pub depth_bottom_cm: f64,
    pub label: String,
    pub age: AgeRange,
}

#[derive(Debug, Clone)]
pub struct SampleCounts {
    pub code: String,
    /// taxon_code -> None(缺失) / Some(n)，n 可为 0（结构零）
    pub counts: BTreeMap<String, Option<i64>>,
}

#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub taxa: Vec<Taxon>,
    pub samples: Vec<SampleAges>,
    pub rows: Vec<SampleCounts>,
    pub mixed_layers: Vec<MixedLayer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Counts,
    Proportion,
    Hellinger,
}

impl Transform {
    pub fn key(self) -> &'static str {
        match self {
            Transform::Counts => "counts",
            Transform::Proportion => "proportion",
            Transform::Hellinger => "hellinger",
        }
    }
}

pub fn r5(x: f64) -> f64 {
    (x * 100_000.0).round() / 100_000.0
}

/// 样本深度中点。沉积芯：深（数值大）= 老。
pub fn depth_mid(s: &SampleAges) -> f64 {
    (s.depth_top_cm + s.depth_bottom_cm) / 2.0
}

/// 分母集合：分类单元 -> 是否进入孢粉总和。
pub fn denominator_flags(taxa: &[Taxon], groups: &Option<Vec<String>>) -> BTreeMap<String, bool> {
    taxa.iter()
        .map(|t| {
            let in_sum = match groups {
                Some(gs) => gs.iter().any(|g| g == &t.eco_group),
                None => t.default_sum != 0,
            };
            (t.code.clone(), in_sum)
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct SampleComputed {
    pub code: String,
    pub label: String,
    pub depth_top_cm: f64,
    pub depth_bottom_cm: f64,
    pub young: f64,
    pub old: f64,
    pub included: bool,
    pub exclude_reason: Option<String>,
    pub denom_sum: i64,
    pub mixed: bool,
    /// taxon_code -> {count:"missing"|0|n, denom:bool, prop:f|null}
    pub taxa: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct BlockComputed {
    pub index: usize,
    pub sample_codes: Vec<String>,
    pub depth_top_cm: f64,
    pub depth_bottom_cm: f64,
    pub denom_total: i64,
    pub totals: BTreeMap<String, i64>,
    pub observed: BTreeMap<String, bool>,
    pub props: BTreeMap<String, f64>,
}

/// 逐样本计算原计数、分母总和与比例（零总计数不生成全零百分比）。
pub fn compute_samples(ds: &Dataset, req: &RunRequest) -> Vec<SampleComputed> {
    let flags = denominator_flags(&ds.taxa, &req.denominator_groups);
    let mut out = Vec::new();

    let mut order: Vec<&SampleAges> = ds.samples.iter().collect();
    order.sort_by(|a, b| {
        depth_mid(a)
            .partial_cmp(&depth_mid(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for s in &order {
        let row = ds
            .rows
            .iter()
            .find(|r| r.code == s.code)
            .cloned()
            .unwrap_or(SampleCounts {
                code: s.code.clone(),
                counts: BTreeMap::new(),
            });

        let mut denom_sum: i64 = 0;
        let mut taxon_map = BTreeMap::new();
        for t in &ds.taxa {
            let in_denom = *flags.get(&t.code).unwrap_or(&false);
            let count = row.counts.get(&t.code).copied().flatten();
            if let Some(n) = count {
                if in_denom {
                    denom_sum += n;
                }
            }
            let count_json = match row.counts.get(&t.code) {
                None | Some(None) => json!("missing"),
                Some(Some(n)) => json!(n),
            };
            taxon_map.insert(
                t.code.clone(),
                json!({
                    "code": t.code,
                    "name": t.name,
                    "eco_group": t.eco_group,
                    "denominator": in_denom,
                    "count": count_json,
                    "prop": None::<f64>,
                }),
            );
        }

        // 关键口径：总计数为零时不生成全零百分比（比例留空，而非 0）。
        let mut zero_total = true;
        if denom_sum > 0 {
            zero_total = false;
            for t in &ds.taxa {
                let in_denom = *flags.get(&t.code).unwrap_or(&false);
                if !in_denom {
                    continue;
                }
                // 仅在有观测（非缺失）时给出比例；缺失保持 null，结构零给 0。
                if let Some(Some(n)) = row.counts.get(&t.code) {
                    if let Some(obj) = taxon_map.get_mut(&t.code) {
                        obj["prop"] = json!(r5(*n as f64 / denom_sum as f64));
                    }
                }
            }
        }

        let mixed = ds.mixed_layers.iter().any(|m| m.contains(depth_mid(s)));

        let (included, reason) = if zero_total {
            (false, Some("zero_total".to_string()))
        } else if denom_sum < req.min_sum {
            (false, Some(format!("low_total(<{})", req.min_sum)))
        } else {
            (true, None)
        };

        out.push(SampleComputed {
            code: s.code.clone(),
            label: s.label.clone(),
            depth_top_cm: s.depth_top_cm,
            depth_bottom_cm: s.depth_bottom_cm,
            young: s.age.young,
            old: s.age.old,
            included,
            exclude_reason: reason,
            denom_sum,
            mixed,
            taxa: taxon_map,
        });
    }
    out
}

fn aggregate_chunk(taxa: &[Taxon], chunk: &[&SampleComputed], index: usize) -> BlockComputed {
    let codes: Vec<String> = chunk.iter().map(|s| s.code.clone()).collect();
    let dtop = chunk
        .iter()
        .map(|s| s.depth_top_cm)
        .fold(f64::INFINITY, f64::min);
    let dbot = chunk
        .iter()
        .map(|s| s.depth_bottom_cm)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    let mut observed: BTreeMap<String, bool> = BTreeMap::new();
    let mut denom_total = 0_i64;

    for s in chunk {
        for (code, obj) in &s.taxa {
            if obj["denominator"].as_bool() != Some(true) {
                continue;
            }
            if let Value::Number(n) = &obj["count"] {
                let v = n.as_i64().unwrap_or(0);
                *totals.entry(code.clone()).or_insert(0) += v;
                observed.insert(code.clone(), true);
                denom_total += v;
            } else {
                observed.entry(code.clone()).or_insert(false);
            }
        }
    }

    let mut props = BTreeMap::new();
    if denom_total > 0 {
        for t in taxa {
            if observed.get(&t.code).copied() == Some(true) {
                let v = *totals.get(&t.code).unwrap_or(&0) as f64 / denom_total as f64;
                props.insert(t.code.clone(), v);
            }
        }
    }

    BlockComputed {
        index,
        sample_codes: codes,
        depth_top_cm: dtop,
        depth_bottom_cm: dbot,
        denom_total,
        totals,
        observed,
        props,
    }
}

/// 把纳入样本按深度（浅→深）切成连续的 k 样本区块并聚合计数。
/// 缺失不计入分母，结构零计入；被排除样本形成断档，两侧不跨越断档合并。
pub fn build_blocks(
    taxa: &[Taxon],
    samples: &[SampleComputed],
    block_size: i64,
) -> Vec<BlockComputed> {
    let k = block_size.max(1) as usize;
    let mut blocks = Vec::new();
    let mut chunk: Vec<&SampleComputed> = Vec::new();

    for s in samples {
        if !s.included {
            if !chunk.is_empty() {
                blocks.push(aggregate_chunk(taxa, &chunk, blocks.len()));
                chunk.clear();
            }
            continue;
        }
        chunk.push(s);
        if chunk.len() == k {
            blocks.push(aggregate_chunk(taxa, &chunk, blocks.len()));
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        blocks.push(aggregate_chunk(taxa, &chunk, blocks.len()));
    }
    blocks
}

/// 两区块在共同“有观测”分母分类单元上的 Hellinger 距离（sqrt(p) 欧氏距离 / sqrt(2)）。
/// 仅在两区块都有观测的分类单元上比较，并对该共同集合重新归一化，避免缺失被当成零。
pub fn hellinger(up: &BlockComputed, down: &BlockComputed) -> f64 {
    let common: Vec<&String> = up
        .props
        .keys()
        .filter(|c| down.props.contains_key(*c))
        .collect();
    if common.is_empty() {
        return 0.0;
    }
    let su: f64 = common.iter().map(|c| up.props[*c]).sum();
    let sd: f64 = common.iter().map(|c| down.props[*c]).sum();
    if su <= 0.0 || sd <= 0.0 {
        return 0.0;
    }
    let mut sumsq = 0.0;
    for c in &common {
        let pu = up.props[*c] / su;
        let pd = down.props[*c] / sd;
        let diff = pu.sqrt() - pd.sqrt();
        sumsq += diff * diff;
    }
    (sumsq / 2.0).sqrt()
}

/// 共同观测分类单元上的比例差（down - up，重新归一化）。
pub fn proportion_deltas(up: &BlockComputed, down: &BlockComputed) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let common: Vec<&String> = up
        .props
        .keys()
        .filter(|c| down.props.contains_key(*c))
        .collect();
    let su: f64 = common.iter().map(|c| up.props[*c]).sum();
    let sd: f64 = common.iter().map(|c| down.props[*c]).sum();
    if su <= 0.0 || sd <= 0.0 {
        return out;
    }
    for c in common {
        out.insert(c.clone(), r5(down.props[c] / sd - up.props[c] / su));
    }
    out
}

/// 计数差（down - up，区块总量，供“数量”视图参考）。
pub fn count_deltas(up: &BlockComputed, down: &BlockComputed) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for (c, v) in &down.totals {
        out.insert(c.clone(), v - up.totals.get(c).copied().unwrap_or(0));
    }
    for c in up.totals.keys() {
        out.entry(c.clone()).or_insert(-up.totals[c]);
    }
    out
}

/// 年代峰：相交区间合并；不相交区间必须保留为多个峰（多峰解释，禁止平均成年份点）。
#[derive(Debug, Clone)]
pub struct AgePeak {
    pub young: f64,
    pub old: f64,
    pub sources: Vec<String>,
}

pub fn merge_age_peaks(upper: AgeRange, lower: AgeRange, su: String, sl: String) -> Vec<AgePeak> {
    // 两个相邻锚点样本的年龄区间（cal a BP，数值越大越老；字段为 young/old）。
    // 规整成 [lo, hi]：
    let (u_lo, u_hi) = (upper.young.min(upper.old), upper.young.max(upper.old));
    let (l_lo, l_hi) = (lower.young.min(lower.old), lower.young.max(lower.old));
    // 边界年龄必须同时满足两锚约束，取交集：
    let lo = u_lo.max(l_lo);
    let hi = u_hi.min(l_hi);
    if lo <= hi {
        // 相交/相接 → 合并为一个年代解释（绝不取均值，只保留交集区间）。
        vec![AgePeak {
            young: lo,
            old: hi,
            sources: vec![su, sl],
        }]
    } else {
        // 不相交（存在年代缺口）→ 保留两个候选峰，绝不平均成一个年份点。
        vec![
            AgePeak {
                young: u_lo,
                old: u_hi,
                sources: vec![su],
            },
            AgePeak {
                young: l_lo,
                old: l_hi,
                sources: vec![sl],
            },
        ]
    }
}

pub fn support_label(h: f64) -> &'static str {
    if h >= 0.40 {
        "strong"
    } else if h >= 0.20 {
        "medium"
    } else {
        "weak"
    }
}

fn block_sample_json(samples: &[SampleComputed], b: &BlockComputed) -> Value {
    let members: Vec<&SampleComputed> = b
        .sample_codes
        .iter()
        .filter_map(|c| samples.iter().find(|s| &s.code == c))
        .collect();
    json!({
        "index": b.index,
        "sample_codes": b.sample_codes,
        "depth_top_cm": b.depth_top_cm,
        "depth_bottom_cm": b.depth_bottom_cm,
        "denom_total": b.denom_total,
        "n_samples": members.len(),
        "profile": b.props.iter().map(|(c, p)| json!({"code": c, "prop": r5(*p)})).collect::<Vec<_>>(),
        "counts": b.totals.iter().map(|(c, n)| json!({"code": c, "count": n})).collect::<Vec<_>>(),
    })
}

/// 相邻区块之间的生态带边界候选（含跨排除样本形成的断档边界）。
fn boundary_json(
    taxa: &[Taxon],
    samples: &[SampleComputed],
    up: &BlockComputed,
    down: &BlockComputed,
    age_version: &str,
    mixed_layers: &[MixedLayer],
) -> Value {
    let h = hellinger(up, down);
    let prop_deltas = proportion_deltas(up, down);
    let count_deltas = count_deltas(up, down);

    let by_code: BTreeMap<&str, &Taxon> = taxa.iter().map(|t| (t.code.as_str(), t)).collect();

    // 锚点样本：上区块最深样本与下区块最浅样本（按深度中点）。
    let anchor_of = |b: &BlockComputed, want_deep: bool| -> Option<&SampleComputed> {
        let mut ms: Vec<&SampleComputed> = b
            .sample_codes
            .iter()
            .filter_map(|c| samples.iter().find(|s| &s.code == c))
            .collect();
        ms.sort_by(|a, z| {
            let da = (a.depth_top_cm + a.depth_bottom_cm) / 2.0;
            let dz = (z.depth_top_cm + z.depth_bottom_cm) / 2.0;
            da.partial_cmp(&dz).unwrap_or(std::cmp::Ordering::Equal)
        });
        if want_deep {
            ms.last().copied()
        } else {
            ms.first().copied()
        }
    };
    let upper_anchor = anchor_of(up, true);
    let lower_anchor = anchor_of(down, false);

    // 方向：以比例差绝对值最大的分母分类单元为指示，同时给出组方向。
    let mut taxon_changes: Vec<Value> = Vec::new();
    for (code, d) in &prop_deltas {
        let t = by_code.get(code.as_str());
        taxon_changes.push(json!({
            "code": code,
            "name": t.map(|x| x.name.as_str()).unwrap_or(code),
            "eco_group": t.map(|x| x.eco_group.as_str()).unwrap_or(""),
            "delta_prop": d,
            "delta_count": count_deltas.get(code).copied().unwrap_or(0),
            "direction": if *d > 0.005 { "up_increase" } else if *d < -0.005 { "down_increase" } else { "stable" },
        }));
    }
    taxon_changes.sort_by(|a, b| {
        b["delta_prop"]
            .as_f64()
            .unwrap_or(0.0)
            .abs()
            .partial_cmp(&a["delta_prop"].as_f64().unwrap_or(0.0).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 组级差异（共同集合重新归一化后的比例求和差）。
    let mut group_up: BTreeMap<String, f64> = BTreeMap::new();
    let mut group_down: BTreeMap<String, f64> = BTreeMap::new();
    for (code, p) in &up.props {
        if let Some(t) = by_code.get(code.as_str()) {
            *group_up.entry(t.eco_group.clone()).or_insert(0.0) += *p;
        }
    }
    for (code, p) in &down.props {
        if let Some(t) = by_code.get(code.as_str()) {
            *group_down.entry(t.eco_group.clone()).or_insert(0.0) += *p;
        }
    }
    let mut groups: BTreeMap<String, bool> = BTreeMap::new();
    for g in group_up.keys().chain(group_down.keys()) {
        groups.insert(g.clone(), true);
    }
    let mut group_changes: Vec<Value> = groups
        .keys()
        .map(|g| {
            let d = group_down.get(g).copied().unwrap_or(0.0) - group_up.get(g).copied().unwrap_or(0.0);
            json!({
                "eco_group": g,
                "upper_prop": r5(group_up.get(g).copied().unwrap_or(0.0)),
                "lower_prop": r5(group_down.get(g).copied().unwrap_or(0.0)),
                "delta_prop": r5(d),
                "direction": if d > 0.01 { "up_increase" } else if d < -0.01 { "down_increase" } else { "stable" },
            })
        })
        .collect();
    group_changes.sort_by(|a, b| {
        b["delta_prop"]
            .as_f64()
            .unwrap_or(0.0)
            .abs()
            .partial_cmp(&a["delta_prop"].as_f64().unwrap_or(0.0).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 指示方向（变化最大组）。
    let leading_direction = group_changes
        .first()
        .and_then(|v| v["eco_group"].as_str().map(|s| s.to_string()))
        .map(|g| {
            let d = group_changes[0]["delta_prop"].as_f64().unwrap_or(0.0);
            format!(
                "向{}偏移（{}）",
                g,
                if d > 0.0 {
                    "下伏增加"
                } else {
                    "上覆增加"
                }
            )
        })
        .unwrap_or_else(|| "组成稳定".to_string());

    // 年代多峰。
    let mut peaks_json: Vec<Value> = Vec::new();
    let mut multi_modal = false;
    let boundary_depth = (up.depth_bottom_cm + down.depth_top_cm) / 2.0;
    if let (Some(ua), Some(la)) = (upper_anchor, lower_anchor) {
        let peaks = merge_age_peaks(
            AgeRange::new(ua.young, ua.old),
            AgeRange::new(la.young, la.old),
            ua.code.clone(),
            la.code.clone(),
        );
        multi_modal = peaks.len() > 1;
        for p in &peaks {
            peaks_json.push(json!({
                "young": r5(p.young),
                "old": r5(p.old),
                "source_samples": p.sources,
            }));
        }
    }

    // 混层：锚点样本落在混层范围 → 不能作为精确边界锦标，但保留方向。
    let in_mixed = |s: Option<&SampleComputed>| -> bool {
        match s {
            Some(sc) => mixed_layers
                .iter()
                .any(|m| m.contains((sc.depth_top_cm + sc.depth_bottom_cm) / 2.0)),
            None => false,
        }
    };
    let imprecise = in_mixed(upper_anchor) || in_mixed(lower_anchor);
    let mut mixed_notes: Vec<String> = Vec::new();
    if imprecise {
        for m in mixed_layers {
            if m.contains(boundary_depth) || in_mixed(upper_anchor) || in_mixed(lower_anchor) {
                mixed_notes.push(format!(
                    "混层 {}–{} cm：{}",
                    m.depth_top_cm, m.depth_bottom_cm, m.note
                ));
            }
        }
    }

    json!({
        "boundary_id": format!("B{}->B{}", up.index, down.index),
        "depth_cm": r5(boundary_depth),
        "upper_block": up.index,
        "lower_block": down.index,
        "upper_samples": up.sample_codes,
        "lower_samples": down.sample_codes,
        "gap": !samples_contiguous(samples, up, down),
        "hellinger": r5(h),
        "support": support_label(h),
        "direction": leading_direction,
        "group_changes": group_changes,
        "taxon_changes_top": taxon_changes.iter().take(6).cloned().collect::<Vec<_>>(),
        "imprecise": imprecise,
        "mixed_notes": mixed_notes,
        "age_model_version": age_version,
        "age_peaks": peaks_json,
        "multi_modal": multi_modal,
    })
}

fn samples_contiguous(
    samples: &[SampleComputed],
    up: &BlockComputed,
    down: &BlockComputed,
) -> bool {
    // 在纳入序列中，如果两区块之间存在被排除样本，则为断档。
    let depth_mid_of = |code: &str| -> Option<f64> {
        samples
            .iter()
            .find(|s| s.code == code)
            .map(|s| (s.depth_top_cm + s.depth_bottom_cm) / 2.0)
    };
    let Some(d_deep) = up
        .sample_codes
        .iter()
        .filter_map(|c| depth_mid_of(c))
        .fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.max(x)))
        })
    else {
        return true;
    };
    let Some(d_shallow) = down
        .sample_codes
        .iter()
        .filter_map(|c| depth_mid_of(c))
        .fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.min(x)))
        })
    else {
        return true;
    };
    !samples.iter().any(|s| {
        !s.included && {
            let dm = (s.depth_top_cm + s.depth_bottom_cm) / 2.0;
            dm > d_deep && dm < d_shallow
        }
    })
}

/// 固定词表指纹：一次分带内的分类单元词表快照。
pub fn vocab_fingerprint(taxa: &[Taxon]) -> String {
    let mut s = String::new();
    for t in taxa {
        s.push_str(&format!(
            "{}|{}|{}|{};",
            t.code, t.name, t.eco_group, t.default_sum
        ));
    }
    let digest = simple_hash(s.as_bytes());
    format!("vocab-v1-{:016x}", digest)
}

fn simple_hash(bytes: &[u8]) -> u64 {
    // FNV-1a 64，确定性即可，不依赖外部 crate。
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 执行一次完整分带分析，返回可直接序列化/持久化的 JSON 结果。
pub fn analyze(ds: &Dataset, req: &RunRequest, run_id: i64, created_at: &str) -> Value {
    // 校验年代模型存在
    let _ = req.age_model_version.clone();

    let samples = compute_samples(ds, req);
    let blocks = build_blocks(&ds.taxa, &samples, req.block_size);

    let sample_json: Vec<Value> = samples
        .iter()
        .map(|s| {
            let taxa: Vec<Value> = s.taxa.values().cloned().collect();
            json!({
                "code": s.code,
                "label": s.label,
                "depth_top_cm": s.depth_top_cm,
                "depth_bottom_cm": s.depth_bottom_cm,
                "depth_cm": r5((s.depth_top_cm + s.depth_bottom_cm) / 2.0),
                "age_young": r5(s.young),
                "age_old": r5(s.old),
                "denom_sum": s.denom_sum,
                "included": s.included,
                "exclude_reason": s.exclude_reason,
                "mixed": s.mixed,
                "zero_total": s.denom_sum == 0,
                "taxa": taxa,
            })
        })
        .collect();

    let mut boundaries: Vec<Value> = Vec::new();
    for w in blocks.windows(2) {
        boundaries.push(boundary_json(
            &ds.taxa,
            &samples,
            &w[0],
            &w[1],
            &req.age_model_version,
            &ds.mixed_layers,
        ));
    }
    // 支持度降序，再按深度降序（老→新更利于生态阅读），保持确定性。
    boundaries.sort_by(|a, b| {
        b["hellinger"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["hellinger"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b["depth_cm"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&a["depth_cm"].as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let denom_groups: Vec<String> = match &req.denominator_groups {
        Some(gs) => gs.clone(),
        None => ds
            .taxa
            .iter()
            .filter(|t| t.default_sum != 0)
            .map(|t| (t.eco_group.clone(), ()))
            .collect::<BTreeMap<String, ()>>()
            .into_keys()
            .collect(),
    };

    json!({
        "run_id": run_id,
        "created_at": created_at,
        "label": req.normalized_label(),
        "config": {
            "denominator_groups": denom_groups,
            "min_sum": req.min_sum,
            "block_size": req.block_size,
            "age_model_version": req.age_model_version,
        },
        "vocab_fingerprint": vocab_fingerprint(&ds.taxa),
        "transforms": {
            "counts": "原始计数（结构零=0；缺失=missing，不参与分母）",
            "proportion": "相对比例 = 分母组计数 / 分母总和；总计数为零时不生成比例",
            "hellinger": "方差稳定变换 sqrt(p)，Hellinger 距离衡量组间差异",
        },
        "samples": sample_json,
        "blocks": blocks.iter().map(|b| block_sample_json(&samples, b)).collect::<Vec<_>>(),
        "boundaries": boundaries,
    })
}

/// 供页面在同一结果上切换数量/比例/Hellinger：对单个样本返回三种视图。
pub fn sample_transform_views(s: &Value, transform: Transform) -> Vec<Value> {
    s["taxa"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    let base = json!({
                        "code": t["code"],
                        "name": t["name"],
                        "eco_group": t["eco_group"],
                        "denominator": t["denominator"],
                        "missing": t["count"] == json!("missing"),
                        "count": if t["count"] == json!("missing") { Value::Null } else { t["count"].clone() },
                        "prop": t["prop"].clone(),
                    });
                    let view_value = match transform {
                        Transform::Counts => {
                            if t["count"] == json!("missing") {
                                Value::Null
                            } else {
                                t["count"].clone()
                            }
                        }
                        Transform::Proportion => t["prop"].clone(),
                        Transform::Hellinger => t["prop"]
                            .as_f64()
                            .map(|p| r5(p.sqrt()))
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    };
                    let mut o = base;
                    o["view"] = view_value;
                    o["transform"] = json!(transform.key());
                    o
                })
                .collect()
        })
        .unwrap_or_default()
}
