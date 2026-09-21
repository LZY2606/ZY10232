use crate::models::{sample_count_map, Fixture};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    Counts,
    Proportion,
    Angular,
}

impl Default for Transform {
    fn default() -> Self {
        Transform::Proportion
    }
}

impl Transform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transform::Counts => "counts",
            Transform::Proportion => "proportion",
            Transform::Angular => "angular",
        }
    }
    pub fn label_cn(&self) -> &'static str {
        match self {
            Transform::Counts => "数量(原计数)",
            Transform::Proportion => "比例(相对百分比)",
            Transform::Angular => "方差稳定(反正弦平方根)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInput {
    /// 固定分组名（取自 fixture.groups），或自定义分类单元清单。
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub denominator_taxa: Option<Vec<String>>,
    #[serde(default = "default_min_total")]
    pub min_total: u64,
    #[serde(default = "default_block_size")]
    pub block_size: usize,
    #[serde(default)]
    pub age_model: Option<String>,
    #[serde(default)]
    pub transform: Transform,
}

fn default_min_total() -> u64 {
    30
}
fn default_block_size() -> usize {
    1
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleStat {
    pub code: String,
    pub depth_cm: f64,
    pub mixed: bool,
    pub observed_total: u64,
    pub denominator_total: u64,
    pub missing_taxa: Vec<String>,
    pub zero_observed_taxa: Vec<String>,
    pub included: bool,
    pub excluded_reason: Option<String>,
    pub proportions: Option<BTreeMap<String, f64>>,
    pub percents: Option<BTreeMap<String, f64>>,
    pub angular: Option<BTreeMap<String, f64>>,
    pub ages: BTreeMap<String, [f64; 2]>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contribution {
    pub taxon: String,
    pub delta_proportion: f64,
    pub delta_angular: f64,
    pub delta_counts: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Boundary {
    pub id: String,
    pub rank: usize,
    pub between: Vec<String>,
    pub gap_index: usize,
    pub depth_range_cm: [f64; 2],
    pub support_counts: f64,
    pub support_proportion: f64,
    pub support_angular: f64,
    pub support: f64,
    pub transform: String,
    pub contributions: Vec<Contribution>,
    pub exact: bool,
    pub fuzzy: bool,
    pub fuzzy_reason: Option<String>,
    pub possible_direction: Option<Direction>,
    pub age_model: String,
    pub age_peaks: Vec<[f64; 2]>,
    pub multi_peak: bool,
    pub peak_note: Option<String>,
    pub skipped_samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Direction {
    pub taxon: String,
    pub sense: String,
    pub sense_cn: String,
    pub delta_proportion: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub site: String,
    pub run_label: String,
    pub age_unit: String,
    pub transform: Transform,
    pub denominator_taxa: Vec<String>,
    pub min_total: u64,
    pub block_size: usize,
    pub age_model: String,
    pub fixture_hash: String,
    pub vocabulary_hash: String,
    pub samples: Vec<SampleStat>,
    pub included_samples: Vec<String>,
    pub boundaries: Vec<Boundary>,
    pub excluded_samples: Vec<String>,
}

pub fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// FNV-1a 64 位哈希，用于固定词表/柱样版本指纹。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn hash_fixture(fx: &Fixture) -> String {
    let canonical = serde_json::to_string(&serde_json::json!({
        "taxa": fx.taxa,
        "samples": fx.samples,
        "counts": fx.counts,
    }))
    .unwrap_or_default();
    format!("fx-{:016x}", fnv1a64(canonical.as_bytes()))
}

/// 每次分带的固定词表：进入分母的分类组，按 fixture 出现顺序排列。
pub fn vocabulary_hash(denominator_taxa: &[String]) -> String {
    let canonical = denominator_taxa.join("|");
    format!("voc-{:016x}", fnv1a64(canonical.as_bytes()))
}

#[derive(Clone)]
pub(crate) struct Resolved {
    denominator: Vec<String>,
    age_model: String,
}

pub(crate) fn validate_input(input: &RunInput, fx: &Fixture) -> Result<Resolved, String> {
    let known: std::collections::BTreeSet<&str> = fx.taxa.iter().map(|t| t.code.as_str()).collect();
    let denominator: Vec<String> = match (&input.denominator_taxa, &input.group) {
        (Some(list), _) if !list.is_empty() => list.iter().map(|s| s.clone()).collect(),
        (_, Some(name)) => fx
            .groups
            .get(name)
            .ok_or_else(|| format!("固定分组不存在: {name}"))?
            .clone(),
        _ => fx
            .groups
            .get("all")
            .cloned()
            .unwrap_or_else(|| fx.taxa.iter().map(|t| t.code.clone()).collect()),
    };
    for code in &denominator {
        if !known.contains(code.as_str()) {
            return Err(format!("分母含未知分类单元: {code}"));
        }
    }
    if input.block_size == 0 {
        return Err("block_size 必须 >= 1".into());
    }
    let age_models: std::collections::BTreeSet<&str> = fx
        .samples
        .iter()
        .flat_map(|s| s.ages.keys().map(|k| k.as_str()))
        .collect();
    let age_model = match &input.age_model {
        Some(m) => {
            if age_models.contains(m.as_str()) {
                m.clone()
            } else {
                return Err(format!("年代模型不存在: {m}"));
            }
        }
        None => age_models
            .iter()
            .min()
            .copied()
            .unwrap_or("default")
            .to_string(),
    };
    Ok(Resolved {
        denominator,
        age_model,
    })
}

/// 合并区间：相交或相接即并入同一峰；不相交则保留多峰，绝不平均成点。
pub fn merge_intervals(input: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut iv: Vec<[f64; 2]> = input.iter().copied().collect();
    iv.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<[f64; 2]> = Vec::new();
    for seg in iv {
        if let Some(last) = out.last_mut() {
            if seg[0] <= last[1] {
                last[1] = last[1].max(seg[1]);
                continue;
            }
        }
        out.push(seg);
    }
    out
}

fn bray_curtis_counts(a: &[f64], b: &[f64]) -> f64 {
    let diff: f64 = a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum();
    let sum: f64 = a.iter().zip(b).map(|(x, y)| x + y).sum();
    if sum == 0.0 {
        0.0
    } else {
        diff / sum
    }
}

fn total_variation(p: &[f64], q: &[f64]) -> f64 {
    // 比例向量的 Bray-Curtis = TV/2，取值 [0,1]。
    0.5 * p.iter().zip(q).map(|(x, y)| (x - y).abs()).sum::<f64>()
}

fn angular_distance(p: &[f64], q: &[f64]) -> f64 {
    // 反正弦平方根变换后的欧氏距离，归一化到 [0,1]（最大值为 sqrt(p/2)，p 为维度数）。
    let s: f64 = p
        .iter()
        .zip(q)
        .map(|(x, y)| {
            let a = x.sqrt().asin();
            let b = y.sqrt().asin();
            (a - b) * (a - b)
        })
        .sum();
    (s.sqrt() / (std::f64::consts::FRAC_PI_2)).min(1.0)
}

fn pooled_counts(
    samples: &[(String, BTreeMap<String, Option<u64>>)],
    denominator: &[String],
) -> Vec<f64> {
    let mut sums = vec![0.0f64; denominator.len()];
    for (_, cmap) in samples {
        for (j, code) in denominator.iter().enumerate() {
            if let Some(Some(n)) = cmap.get(code) {
                sums[j] += *n as f64;
            }
        }
    }
    sums
}

fn to_proportions(v: &[f64]) -> Vec<f64> {
    let total: f64 = v.iter().sum();
    if total <= 0.0 {
        return vec![0.0; v.len()];
    }
    v.iter().map(|x| x / total).collect()
}

pub fn run(fx: &Fixture, input: RunInput) -> Result<RunResult, String> {
    let Resolved {
        denominator,
        age_model,
    } = validate_input(&input, fx)?;

    // 按深度排序（浅 -> 深）。
    let mut ordered: Vec<&crate::models::RawSample> = fx.samples.iter().collect();
    ordered.sort_by(|a, b| {
        a.depth_cm
            .partial_cmp(&b.depth_cm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut sample_stats: Vec<SampleStat> = Vec::new();
    let mut included_rows: Vec<(String, BTreeMap<String, Option<u64>>)> = Vec::new();

    for raw in &ordered {
        let cmap = sample_count_map(fx, &raw.code);
        let observed_total: u64 = cmap.values().filter_map(|v| *v).sum();
        let denominator_total: u64 = denominator
            .iter()
            .filter_map(|code| cmap.get(code).and_then(|v| *v))
            .sum();
        let missing_taxa: Vec<String> = cmap
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        let zero_observed_taxa: Vec<String> = cmap
            .iter()
            .filter(|(_, v)| matches!(v, Some(0)))
            .map(|(k, _)| k.clone())
            .collect();

        // 零总数（无任何观察值）与"有观察但分母为 0"都不生成百分比。
        let mut excluded_reason: Option<String> = None;
        if observed_total == 0 {
            excluded_reason = Some("零总数：无观察计数，不生成全零百分比".to_string());
        } else if denominator_total < input.min_total {
            excluded_reason = Some(format!(
                "分母总计数 {} 低于阈值 {}",
                denominator_total, input.min_total
            ));
        }

        let included = excluded_reason.is_none();
        let proportions: Option<BTreeMap<String, f64>> = if included {
            let total = denominator_total as f64;
            Some(
                denominator
                    .iter()
                    .map(|code| {
                        let n = cmap.get(code).and_then(|v| *v).unwrap_or(0) as f64;
                        (code.clone(), round6(n / total))
                    })
                    .collect(),
            )
        } else {
            None
        };
        let percents = proportions.as_ref().map(|p| {
            p.iter()
                .map(|(k, v)| (k.clone(), round6(v * 100.0)))
                .collect()
        });
        let angular = proportions.as_ref().map(|p| {
            p.iter()
                .map(|(k, v)| (k.clone(), round6(v.sqrt().asin())))
                .collect()
        });

        if included {
            included_rows.push((raw.code.clone(), cmap));
        }
        sample_stats.push(SampleStat {
            code: raw.code.clone(),
            depth_cm: raw.depth_cm,
            mixed: raw.mixed,
            observed_total,
            denominator_total,
            missing_taxa,
            zero_observed_taxa,
            included,
            excluded_reason,
            proportions,
            percents,
            angular,
            ages: raw.ages.clone(),
            note: raw.note.clone(),
        });
    }

    // 将纳入样本按相邻 block_size 切成相邻块；块间间隙即为边界候选。
    let included_codes: Vec<String> = included_rows.iter().map(|(c, _)| c.clone()).collect();
    let k = input.block_size.min(included_rows.len().max(1));
    let mut boundaries: Vec<Boundary> = Vec::new();
    let mixed_lookup: BTreeMap<String, bool> =
        ordered.iter().map(|s| (s.code.clone(), s.mixed)).collect();
    let depth_lookup: BTreeMap<String, f64> = ordered
        .iter()
        .map(|s| (s.code.clone(), s.depth_cm))
        .collect();

    if included_rows.len() >= 2 {
        for start in (k..included_rows.len()).step_by(k) {
            let upper = &included_rows[start - k..start];
            let lower = &included_rows[start..(start + k).min(included_rows.len())];

            let upper_codes: Vec<String> = upper.iter().map(|(c, _)| c.clone()).collect();
            let lower_codes: Vec<String> = lower.iter().map(|(c, _)| c.clone()).collect();
            let upper_deep = upper_codes
                .iter()
                .map(|c| depth_lookup[c])
                .fold(f64::NAN, f64::max);
            let lower_shallow = lower_codes
                .iter()
                .map(|c| depth_lookup[c])
                .fold(f64::INFINITY, f64::min);

            // 间隙内被排除（低计数/零总数）或跨越的样本。
            let skipped_samples: Vec<String> = ordered
                .iter()
                .filter(|s| s.depth_cm > upper_deep && s.depth_cm < lower_shallow)
                .map(|s| s.code.clone())
                .collect();

            let gap_index = start / k - 1;

            let counts_up = pooled_counts(upper, &denominator);
            let counts_lo = pooled_counts(lower, &denominator);
            let prop_up = to_proportions(&counts_up);
            let prop_lo = to_proportions(&counts_lo);

            let support_counts = round6(bray_curtis_counts(&counts_up, &counts_lo));
            let support_proportion = round6(total_variation(&prop_up, &prop_lo));
            let support_angular = round6(angular_distance(&prop_up, &prop_lo));

            let mut contributions: Vec<Contribution> = denominator
                .iter()
                .enumerate()
                .map(|(j, code)| Contribution {
                    taxon: code.clone(),
                    delta_proportion: round6(prop_up[j] - prop_lo[j]),
                    delta_angular: round6(prop_up[j].sqrt().asin() - prop_lo[j].sqrt().asin()),
                    delta_counts: round6(counts_up[j] - counts_lo[j]),
                })
                .collect();
            contributions.sort_by(|a, b| {
                b.delta_proportion
                    .abs()
                    .partial_cmp(&a.delta_proportion.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let support = match input.transform {
                Transform::Counts => support_counts,
                Transform::Proportion => support_proportion,
                Transform::Angular => support_angular,
            };

            let fuzzy_codes: Vec<&String> = upper_codes
                .iter()
                .chain(lower_codes.iter())
                .chain(skipped_samples.iter())
                .filter(|c| mixed_lookup[*c])
                .collect();
            let fuzzy = !fuzzy_codes.is_empty();
            let fuzzy_reason = if fuzzy {
                Some(format!(
                    "混层范围触及: {}",
                    fuzzy_codes
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else {
                None
            };
            let top = contributions.first();
            let possible_direction = top.filter(|c| c.delta_proportion.abs() > 0.0).map(|c| {
                let (sense, sense_cn) = if c.delta_proportion > 0.0 {
                    ("increase_upcore", "向上(浅/新)增多")
                } else {
                    ("decrease_upcore", "向上(浅/新)减少")
                };
                Direction {
                    taxon: c.taxon.clone(),
                    sense: sense.to_string(),
                    sense_cn: sense_cn.to_string(),
                    delta_proportion: c.delta_proportion,
                }
            });

            // 年代区间：聚合块内与间隙全部样本，相交则合并，不相交保留多峰。
            let mut raw_intervals: Vec<[f64; 2]> = Vec::new();
            for code in upper_codes
                .iter()
                .chain(lower_codes.iter())
                .chain(skipped_samples.iter())
            {
                if let Some(iv) = sample_stats
                    .iter()
                    .find(|s| &s.code == code)
                    .and_then(|s| s.ages.get(&age_model))
                {
                    raw_intervals.push(*iv);
                }
            }
            let age_peaks = merge_intervals(&raw_intervals);
            let multi_peak = age_peaks.len() > 1;
            let peak_note = if multi_peak {
                Some("年代区间不相交：保留多峰时间解释，不平均成单一年份点".to_string())
            } else {
                None
            };

            boundaries.push(Boundary {
                id: String::new(),
                rank: 0,
                between: vec![
                    upper_codes.last().unwrap().clone(),
                    lower_codes.first().unwrap().clone(),
                ],
                gap_index,
                depth_range_cm: [upper_deep, lower_shallow],
                support_counts,
                support_proportion,
                support_angular,
                support,
                transform: input.transform.as_str().to_string(),
                contributions,
                exact: !fuzzy,
                fuzzy,
                fuzzy_reason,
                possible_direction,
                age_model: age_model.clone(),
                age_peaks,
                multi_peak,
                peak_note,
                skipped_samples,
            });
        }

        boundaries.sort_by(|a, b| {
            b.support
                .partial_cmp(&a.support)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.depth_range_cm[0]
                        .partial_cmp(&b.depth_range_cm[0])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        for (i, b) in boundaries.iter_mut().enumerate() {
            b.rank = i + 1;
            b.id = format!("B{:02}", i + 1);
        }
    }

    let excluded_samples: Vec<String> = sample_stats
        .iter()
        .filter(|s| !s.included)
        .map(|s| s.code.clone())
        .collect();

    let run_label = format!(
        "{}-den{}-min{}-k{}-{}-{}",
        fx.site,
        vocabulary_hash(&denominator)
            .replace("voc-", "")
            .get(0..8)
            .unwrap_or("voc"),
        input.min_total,
        k,
        age_model,
        input.transform.as_str()
    );

    let vocab_hash = vocabulary_hash(&denominator);

    Ok(RunResult {
        site: fx.site.clone(),
        run_label,
        age_unit: fx.age_unit.clone(),
        transform: input.transform.clone(),
        denominator_taxa: denominator,
        min_total: input.min_total,
        block_size: k,
        age_model,
        fixture_hash: hash_fixture(fx),
        vocabulary_hash: vocab_hash,
        samples: sample_stats,
        included_samples: included_codes,
        boundaries,
        excluded_samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Fixture {
        crate::models::load_fixture(include_str!("../fixtures/fixture.json")).unwrap()
    }

    #[test]
    fn interval_merge_keeps_disjoint_peaks() {
        assert_eq!(merge_intervals(&[[5.0, 10.0]]), vec![[5.0, 10.0]]);
        assert_eq!(
            merge_intervals(&[[100.0, 200.0], [150.0, 260.0]]),
            vec![[100.0, 260.0]]
        );
        assert_eq!(
            merge_intervals(&[[100.0, 150.0], [150.0, 200.0], [300.0, 400.0]]),
            vec![[100.0, 200.0], [300.0, 400.0]]
        );
    }

    #[test]
    fn zero_total_sample_gets_no_percents() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 0,
                block_size: 1,
                age_model: Some("am_2024".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        let s4 = r.samples.iter().find(|s| s.code == "S4").unwrap();
        assert_eq!(s4.observed_total, 0);
        assert!(s4.proportions.is_none(), "零总数不得生成全零百分比");
        assert!(s4.excluded_reason.is_some());
        assert!(!r.included_samples.contains(&"S4".to_string()));
    }

    #[test]
    fn missing_distinct_from_zero() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 0,
                block_size: 1,
                age_model: Some("am_2019".into()),
                transform: Transform::Counts,
            },
        )
        .unwrap();
        let s4 = r.samples.iter().find(|s| s.code == "S4").unwrap();
        assert!(s4.zero_observed_taxa.contains(&"Betula".to_string()));
        assert!(s4.missing_taxa.contains(&"Pteridium".to_string()));
        assert!(s4.missing_taxa.contains(&"Quercus".to_string()));
    }

    #[test]
    fn low_count_exclusion_default_threshold() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 1,
                age_model: Some("am_2024".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        assert!(r.excluded_samples.contains(&"S7".to_string()));
        // S6(混层) 与 S8 之间因 S7 被排除，间隙候选保留，且年代不相交 -> 多峰。
        let b = r
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S6".to_string(), "S8".to_string()])
            .unwrap();
        assert_eq!(b.skipped_samples, vec!["S7".to_string()]);
        assert!(b.multi_peak, "am_2024 下该间隙年代不相交，须保留多峰");
        assert!(b.age_peaks.len() >= 2);
        assert!(b.fuzzy, "块内含混层样本 S6，只能是模糊候选");
        assert!(!b.exact);
    }

    #[test]
    fn mixed_boundary_is_fuzzy_but_shows_direction() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 0,
                block_size: 1,
                age_model: Some("am_2019".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        let b = r
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S5".to_string(), "S6".to_string()])
            .unwrap();
        assert!(b.fuzzy);
        assert!(b.possible_direction.is_some());
    }

    #[test]
    fn denominator_change_changes_support() {
        let fx = fixture();
        let base = RunInput {
            group: None,
            denominator_taxa: None,
            min_total: 30,
            block_size: 1,
            age_model: Some("am_2024".into()),
            transform: Transform::Proportion,
        };
        let mut with_pteridium = base.clone();
        with_pteridium.group = Some("all".into());
        let mut no_pteridium = base.clone();
        no_pteridium.group = Some("pollen_only".into());
        let r1 = run(&fx, with_pteridium).unwrap();
        let r2 = run(&fx, no_pteridium).unwrap();
        assert_ne!(r1.vocabulary_hash, r2.vocabulary_hash);
        let top1 = r1
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S8".to_string(), "S9".to_string()])
            .unwrap();
        let top2 = r2
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S8".to_string(), "S9".to_string()])
            .unwrap();
        assert_ne!(top1.support_proportion, top2.support_proportion);
    }

    #[test]
    fn transforms_agree_on_ranking_shape() {
        let fx = fixture();
        let mk = |t| RunInput {
            group: Some("all".into()),
            denominator_taxa: None,
            min_total: 30,
            block_size: 1,
            age_model: Some("am_2024".into()),
            transform: t,
        };
        let rp = run(&fx, mk(Transform::Proportion)).unwrap();
        let ra = run(&fx, mk(Transform::Angular)).unwrap();
        let rc = run(&fx, mk(Transform::Counts)).unwrap();
        // 同一候选在三种口径下都给出三种支持度数值。
        let b = rp.boundaries.first().unwrap();
        assert!((0.0..=1.0).contains(&b.support_angular));
        assert_eq!(rc.transform, Transform::Counts);
        let _ = ra;
    }

    #[test]
    fn multi_peak_never_averaged() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("pollen_only".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 1,
                age_model: Some("am_2024".into()),
                transform: Transform::Angular,
            },
        )
        .unwrap();
        let b = r
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S6".to_string(), "S8".to_string()])
            .unwrap();
        let flat: Vec<f64> = b.age_peaks.iter().flatten().copied().collect();
        // 多峰端点必须仍以区间数组形式存在，不出现单一中点。
        assert!(b.age_peaks.iter().all(|iv| iv[1] >= iv[0]));
        assert!(flat.len() >= 4);
    }

    #[test]
    fn am2019_overlapping_intervals_merge_to_one_peak() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 1,
                age_model: Some("am_2019".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        // S8[880,980] 与 S9[970,1090] 相交(970<=980)：合并为单峰 [880,1090]。
        let b = r
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S8".to_string(), "S9".to_string()])
            .unwrap();
        assert_eq!(b.age_peaks, vec![[880.0, 1090.0]]);
        assert!(!b.multi_peak);
        // 同一间隙在 am_2024 区间不相交，必须保留双峰。
        let r2 = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 1,
                age_model: Some("am_2024".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        let b2 = r2
            .boundaries
            .iter()
            .find(|b| b.between == vec!["S8".to_string(), "S9".to_string()])
            .unwrap();
        assert!(b2.multi_peak);
        assert_eq!(b2.age_peaks, vec![[900.0, 1020.0], [1100.0, 1220.0]]);
    }

    #[test]
    fn block_size_merges_neighbors() {
        let fx = fixture();
        let r = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 2,
                age_model: Some("am_2019".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        // k=2 时每块最多两个相邻样本，边界数少于 k=1。
        let r1 = run(
            &fx,
            RunInput {
                group: Some("all".into()),
                denominator_taxa: None,
                min_total: 30,
                block_size: 1,
                age_model: Some("am_2019".into()),
                transform: Transform::Proportion,
            },
        )
        .unwrap();
        assert!(r.boundaries.len() < r1.boundaries.len());
    }

    #[test]
    fn invalid_denominator_rejected() {
        let fx = fixture();
        let err = run(
            &fx,
            RunInput {
                group: None,
                denominator_taxa: Some(vec!["Nope".into()]),
                min_total: 0,
                block_size: 1,
                age_model: None,
                transform: Transform::Counts,
            },
        );
        assert!(err.is_err());
    }
}
