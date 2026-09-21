//! 统计核心单元测试：零/缺失区分、多峰不平均、Hellinger 与分母切换。

use std::collections::BTreeMap;

use pollen_stage::analysis::{
    analyze, compute_samples, denominator_flags, hellinger, merge_age_peaks, BlockComputed,
    Dataset, SampleAges, SampleCounts, Transform,
};
use pollen_stage::models::{AgeRange, MixedLayer, RunRequest, Taxon};

fn taxon(code: &str, group: &str, default_sum: i64) -> Taxon {
    Taxon {
        code: code.to_string(),
        name: code.to_string(),
        eco_group: group.to_string(),
        default_sum,
    }
}

fn sample(code: &str, depth: f64, young: f64, old: f64) -> SampleAges {
    SampleAges {
        code: code.to_string(),
        depth_top_cm: depth,
        depth_bottom_cm: depth + 10.0,
        label: code.to_string(),
        age: AgeRange::new(young, old),
    }
}

fn counts(code: &str, pairs: &[(&str, Option<i64>)]) -> SampleCounts {
    let mut m = BTreeMap::new();
    for (c, n) in pairs {
        m.insert(c.to_string(), *n);
    }
    SampleCounts {
        code: code.to_string(),
        counts: m,
    }
}

#[test]
fn zero_total_generates_no_proportions_but_missing_is_distinct() {
    let taxa = vec![taxon("A", "乔木", 1), taxon("B", "水生", 0)];
    let samples = vec![sample("S1", 0.0, 100.0, 200.0)];
    // A 为结构零 0，B 为缺失 None；总计数为 0。
    let rows = vec![counts("S1", &[("A", Some(0)), ("B", None)])];
    let ds = Dataset {
        taxa,
        samples,
        rows,
        mixed_layers: vec![],
    };
    let req = RunRequest {
        label: None,
        denominator_groups: None,
        min_sum: 0,
        block_size: 1,
        age_model_version: "v".to_string(),
    };
    let computed = compute_samples(&ds, &req);
    let s = &computed[0];
    assert_eq!(s.denom_sum, 0);
    assert_eq!(s.exclude_reason.as_deref(), Some("zero_total"));
    // 关键：零总计数不生成全零百分比（prop 为 null，而不是 0.0）。
    let a = &s.taxa["A"];
    assert_eq!(a["count"], serde_json::json!(0));
    assert!(a["prop"].is_null(), "结构零在零总和下不得产生 0 比例");
    let b = &s.taxa["B"];
    assert_eq!(b["count"], serde_json::json!("missing"));
    assert!(b["prop"].is_null());
}

#[test]
fn missing_counts_do_not_inflate_denominator() {
    let taxa = vec![taxon("A", "乔木", 1), taxon("B", "乔木", 1)];
    let samples = vec![sample("S1", 0.0, 100.0, 200.0)];
    // A=40 有观测；B 缺失。比例应基于有观测集合（这里即 A），总和=40 而非把缺失当 0。
    let rows = vec![counts("S1", &[("A", Some(40)), ("B", None)])];
    let ds = Dataset {
        taxa,
        samples,
        rows,
        mixed_layers: vec![],
    };
    let req = RunRequest {
        label: None,
        denominator_groups: None,
        min_sum: 0,
        block_size: 1,
        age_model_version: "v".to_string(),
    };
    let computed = compute_samples(&ds, &req);
    let s = &computed[0];
    assert_eq!(s.denom_sum, 40);
    assert!((s.taxa["A"]["prop"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert!(s.taxa["B"]["prop"].is_null());
}

#[test]
fn disjoint_ages_keep_two_peaks_never_averaged() {
    // 上锚 S03=800-1000，下锚 S06=4500-5200（倒挂缺段）→ 两峰保留。
    let peaks = merge_age_peaks(
        AgeRange::new(800.0, 1000.0),
        AgeRange::new(4500.0, 5200.0),
        "S03".into(),
        "S06".into(),
    );
    assert_eq!(peaks.len(), 2, "不相交年代必须保留多峰");
    assert_eq!(peaks[0].young, 800.0);
    assert_eq!(peaks[0].old, 1000.0);
    assert_eq!(peaks[1].young, 4500.0);
    assert_eq!(peaks[1].old, 5200.0);

    // 相邻区间：上锚(新) 820-1020，下锚(老) 1000-1400 → 交集 1000-1020，合并。
    let merged = merge_age_peaks(
        AgeRange::new(820.0, 1020.0),
        AgeRange::new(1000.0, 1400.0),
        "U".into(),
        "L".into(),
    );
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].young, 1000.0);
    assert_eq!(merged[0].old, 1020.0);
    // 上锚 800-1000、下锚 1000-1200 恰相接也算合并
    let touching = merge_age_peaks(
        AgeRange::new(800.0, 1000.0),
        AgeRange::new(1000.0, 1200.0),
        "U".into(),
        "L".into(),
    );
    assert_eq!(touching.len(), 1);
    assert_eq!(touching[0].young, 1000.0);
    assert_eq!(touching[0].old, 1000.0);
}

#[test]
fn hellinger_is_zero_for_identical_and_positive_for_turnover() {
    let mk = |map: &[(&str, f64)]| {
        let mut props = BTreeMap::new();
        let mut totals = BTreeMap::new();
        for (c, p) in map {
            props.insert(c.to_string(), *p);
            totals.insert(c.to_string(), (p * 100.0) as i64);
        }
        BlockComputed {
            index: 0,
            sample_codes: vec![],
            depth_top_cm: 0.0,
            depth_bottom_cm: 0.0,
            denom_total: 100,
            totals,
            observed: BTreeMap::new(),
            props,
        }
    };
    let a = mk(&[("A", 0.5), ("B", 0.5)]);
    let b = mk(&[("A", 0.5), ("B", 0.5)]);
    assert!(hellinger(&a, &b).abs() < 1e-12);
    let c = mk(&[("A", 1.0), ("B", 0.0)]);
    assert!(hellinger(&a, &c) > 0.4);
}

#[test]
fn changing_denominator_groups_changes_proportions_and_boundaries() {
    // 三个分类：A/B 乔木，G 陆生草本。上部 A 主导，下部 G 主导。
    let taxa = vec![
        taxon("A", "乔木", 1),
        taxon("B", "乔木", 1),
        taxon("G", "陆生草本", 1),
    ];
    let samples = vec![
        sample("U1", 0.0, 100.0, 200.0),
        sample("U2", 20.0, 300.0, 400.0),
        sample("D1", 40.0, 700.0, 900.0),
        sample("D2", 60.0, 1000.0, 1200.0),
    ];
    let rows = vec![
        counts("U1", &[("A", Some(80)), ("B", Some(10)), ("G", Some(10))]),
        counts("U2", &[("A", Some(70)), ("B", Some(20)), ("G", Some(10))]),
        counts("D1", &[("A", Some(10)), ("B", Some(5)), ("G", Some(85))]),
        counts("D2", &[("A", Some(5)), ("B", Some(5)), ("G", Some(90))]),
    ];
    let ds = Dataset {
        taxa,
        samples,
        rows,
        mixed_layers: vec![],
    };
    let base = RunRequest {
        label: None,
        denominator_groups: None,
        min_sum: 0,
        block_size: 1,
        age_model_version: "v".to_string(),
    };
    let all = analyze(&ds, &base, 1, "t");
    let h_all = all["boundaries"][0]["hellinger"].as_f64().unwrap();

    // 仅乔木分母：上下区块都被 A 主导（G 被剔除），差异应显著缩小。
    let trees = RunRequest {
        denominator_groups: Some(vec!["乔木".to_string()]),
        ..clone_req(&base)
    };
    let only_trees = analyze(&ds, &trees, 2, "t");
    let h_trees = only_trees["boundaries"][0]["hellinger"].as_f64().unwrap();
    assert!(
        h_all > h_trees,
        "切换分母组应改变边界支持度：{h_all} vs {h_trees}"
    );
}

fn clone_req(r: &RunRequest) -> RunRequest {
    RunRequest {
        label: r.label.clone(),
        denominator_groups: r.denominator_groups.clone(),
        min_sum: r.min_sum,
        block_size: r.block_size,
        age_model_version: r.age_model_version.clone(),
    }
}

#[test]
fn mixed_layer_boundary_is_imprecise_but_keeps_direction() {
    let taxa = vec![taxon("A", "乔木", 1), taxon("G", "陆生草本", 1)];
    let samples = vec![
        sample("U", 0.0, 100.0, 200.0),
        sample("M", 100.0, 400.0, 600.0), // 落在混层
        sample("D", 200.0, 1500.0, 1800.0),
    ];
    let rows = vec![
        counts("U", &[("A", Some(90)), ("G", Some(10))]),
        counts("M", &[("A", Some(50)), ("G", Some(50))]),
        counts("D", &[("A", Some(10)), ("G", Some(90))]),
    ];
    let mixed = vec![MixedLayer {
        depth_top_cm: 90.0,
        depth_bottom_cm: 160.0,
        note: "test".to_string(),
    }];
    let ds = Dataset {
        taxa,
        samples,
        rows,
        mixed_layers: mixed,
    };
    let req = RunRequest {
        label: None,
        denominator_groups: None,
        min_sum: 0,
        block_size: 1,
        age_model_version: "v".to_string(),
    };
    let r = analyze(&ds, &req, 1, "t");
    let imp: Vec<_> = r["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["imprecise"].as_bool() == Some(true))
        .collect();
    assert!(!imp.is_empty(), "混层锚点边界必须标记为不精确");
    for b in imp {
        assert!(
            b["direction"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "不精确边界仍须保留方向"
        );
        assert!(!b["mixed_notes"].as_array().unwrap().is_empty());
    }
}

#[test]
fn transform_enum_keys() {
    assert_eq!(Transform::Counts.key(), "counts");
    assert_eq!(Transform::Proportion.key(), "proportion");
    assert_eq!(Transform::Hellinger.key(), "hellinger");
    let _ = denominator_flags(&[taxon("A", "乔木", 1)], &None);
}
