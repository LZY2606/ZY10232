//! 固定演示 fixture（可复放）：低总计数层、全零层、缺失计数、混层区，
//! 以及两个年代模型下一个相交、一个不相交（多峰）的关键边界。

use rusqlite::{params, Connection};

use crate::db::{self, FIXTURE_VERSION};

#[derive(Clone, Copy)]
struct Tax {
    code: &'static str,
    name: &'static str,
    group: &'static str,
    default_sum: i64,
}

/// 固定词表（顺序固定）。
const TAXA: &[Tax] = &[
    Tax {
        code: "PINUS",
        name: "松属",
        group: "乔木",
        default_sum: 1,
    },
    Tax {
        code: "PICEA",
        name: "云杉属",
        group: "乔木",
        default_sum: 1,
    },
    Tax {
        code: "QUERCUS",
        name: "栎属",
        group: "乔木",
        default_sum: 1,
    },
    Tax {
        code: "BETULA",
        name: "桦木属",
        group: "乔木",
        default_sum: 1,
    },
    Tax {
        code: "ALNUS",
        name: "桤木属",
        group: "乔木",
        default_sum: 1,
    },
    Tax {
        code: "ARTEMISIA",
        name: "蒿属",
        group: "陆生草本",
        default_sum: 1,
    },
    Tax {
        code: "CHENOPOD",
        name: "藜科",
        group: "陆生草本",
        default_sum: 1,
    },
    Tax {
        code: "POACEAE",
        name: "禾本科",
        group: "陆生草本",
        default_sum: 1,
    },
    Tax {
        code: "TYPHA",
        name: "香蒲属",
        group: "水生",
        default_sum: 0,
    },
    Tax {
        code: "PTERIDIUM",
        name: "蕨属(孢子)",
        group: "蕨类",
        default_sum: 0,
    },
];

struct Samp {
    code: &'static str,
    top: f64,
    bottom: f64,
    label: &'static str,
    v1: (f64, f64),
    v2: (f64, f64),
    // 计数按 TAXA 顺序；None = 缺失，Some(n) = 结构计数（含 0）
    counts: [Option<i64>; 10],
}

const SAMPLES: &[Samp] = &[
    // 上部：全新世暖期落叶阔叶林
    Samp {
        code: "S01",
        top: 0.0,
        bottom: 10.0,
        label: "0-10cm 林冠表土",
        v1: (100.0, 250.0),
        v2: (120.0, 260.0),
        counts: [
            Some(40),
            Some(5),
            Some(80),
            Some(60),
            Some(50),
            Some(20),
            Some(10),
            Some(25),
            Some(30),
            Some(15),
        ],
    },
    Samp {
        code: "S02",
        top: 20.0,
        bottom: 30.0,
        label: "20-30cm 暖期森林",
        v1: (400.0, 600.0),
        v2: (420.0, 620.0),
        counts: [
            Some(45),
            Some(8),
            Some(75),
            Some(55),
            Some(55),
            Some(18),
            Some(8),
            Some(22),
            None,
            Some(12),
        ],
    },
    Samp {
        code: "S03",
        top: 40.0,
        bottom: 50.0,
        label: "40-50cm 暖期末森林",
        v1: (800.0, 1000.0),
        v2: (820.0, 1020.0),
        counts: [
            Some(50),
            Some(10),
            Some(70),
            Some(50),
            Some(60),
            Some(15),
            Some(5),
            Some(20),
            None,
            Some(10),
        ],
    },
    // 低计数层 + 全零层（默认被排除，形成断档）
    Samp {
        code: "S04",
        top: 60.0,
        bottom: 70.0,
        label: "60-70cm 低计数层",
        v1: (1200.0, 1500.0),
        v2: (1250.0, 1600.0),
        counts: [
            Some(2),
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            Some(1),
            Some(0),
            Some(1),
            Some(0),
            Some(0),
        ],
    },
    Samp {
        code: "S05",
        top: 80.0,
        bottom: 90.0,
        label: "80-90cm 无花粉层",
        v1: (1800.0, 2200.0),
        v2: (2400.0, 3000.0),
        counts: [
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
        ],
    },
    // 下部：晚冰期/冷期草原-云杉。S03 与 S06 的年代在 v1 不相交（多峰），v2 相交（合并）
    Samp {
        code: "S06",
        top: 100.0,
        bottom: 110.0,
        label: "100-110cm 冷期草原",
        v1: (4500.0, 5200.0),
        v2: (1000.0, 1400.0),
        counts: [
            Some(10),
            Some(40),
            Some(5),
            Some(15),
            Some(10),
            Some(70),
            Some(45),
            Some(35),
            Some(2),
            Some(3),
        ],
    },
    Samp {
        code: "S07",
        top: 120.0,
        bottom: 130.0,
        label: "120-130cm 云杉-蒿",
        v1: (5600.0, 6400.0),
        v2: (5800.0, 6500.0),
        counts: [
            Some(8),
            Some(55),
            Some(3),
            Some(10),
            Some(8),
            Some(60),
            Some(40),
            Some(30),
            Some(1),
            Some(2),
        ],
    },
    // 混层区 140-200cm（再搬运，边界不精确，但方向仍显示）
    Samp {
        code: "S08",
        top: 140.0,
        bottom: 150.0,
        label: "140-150cm 混层上部",
        v1: (6800.0, 7800.0),
        v2: (6900.0, 7900.0),
        counts: [
            Some(20),
            Some(30),
            Some(10),
            Some(20),
            Some(15),
            Some(45),
            Some(30),
            Some(40),
            Some(5),
            Some(40),
        ],
    },
    Samp {
        code: "S09",
        top: 180.0,
        bottom: 190.0,
        label: "180-190cm 混层下部",
        v1: (8200.0, 9200.0),
        v2: (8300.0, 9300.0),
        counts: [
            Some(25),
            Some(25),
            Some(15),
            Some(25),
            Some(20),
            Some(40),
            Some(25),
            Some(45),
            Some(8),
            Some(45),
        ],
    },
    // 冷期草原最下部
    Samp {
        code: "S10",
        top: 220.0,
        bottom: 230.0,
        label: "220-230cm 干冷草原",
        v1: (9800.0, 11000.0),
        v2: (9900.0, 11100.0),
        counts: [
            Some(5),
            Some(20),
            Some(2),
            Some(8),
            Some(5),
            Some(85),
            Some(55),
            Some(50),
            Some(0),
            None,
        ],
    },
    Samp {
        code: "S11",
        top: 260.0,
        bottom: 270.0,
        label: "260-270cm 草原",
        v1: (11500.0, 12800.0),
        v2: (11600.0, 12900.0),
        counts: [
            Some(4),
            Some(18),
            Some(1),
            Some(6),
            Some(4),
            Some(90),
            Some(60),
            Some(47),
            Some(0),
            Some(1),
        ],
    },
    Samp {
        code: "S12",
        top: 300.0,
        bottom: 310.0,
        label: "300-310cm 冰期蒿藜峰",
        v1: (13200.0, 14500.0),
        v2: (13300.0, 14600.0),
        counts: [
            Some(3),
            Some(15),
            Some(1),
            Some(5),
            Some(3),
            Some(100),
            Some(70),
            Some(40),
            Some(0),
            Some(1),
        ],
    },
];

const AGE_MODELS: &[(&str, &str)] = &[
    ("age-v1-linear", "v1 线性插值年代模型"),
    ("age-v2-bacon", "v2 Bacon 贝叶斯年代模型"),
];

const MIXED: &[(f64, f64, &str)] = &[(140.0, 200.0, "再搬运/生物扰动混层，边界只可指示方向")];

pub fn wipe(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
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
    )?;
    db::init_schema(conn)
}

pub fn seed(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('fixture_version',?1)",
        params![FIXTURE_VERSION],
    )?;
    for (i, t) in TAXA.iter().enumerate() {
        tx.execute(
            "INSERT INTO taxa(code,name,eco_group,default_sum,sort_order) VALUES(?1,?2,?3,?4,?5)",
            params![t.code, t.name, t.group, t.default_sum, i as i64],
        )?;
    }
    for (i, (v, label)) in AGE_MODELS.iter().enumerate() {
        tx.execute(
            "INSERT INTO age_models(version,label,sort_order) VALUES(?1,?2,?3)",
            params![v, label, i as i64],
        )?;
    }
    for (i, s) in SAMPLES.iter().enumerate() {
        tx.execute(
            "INSERT INTO samples(code,depth_top_cm,depth_bottom_cm,label,sort_order) VALUES(?1,?2,?3,?4,?5)",
            params![s.code, s.top, s.bottom, s.label, i as i64],
        )?;
        tx.execute(
            "INSERT INTO sample_ages(sample_code,age_model_version,young,old) VALUES(?1,?2,?3,?4)",
            params![s.code, AGE_MODELS[0].0, s.v1.0, s.v1.1],
        )?;
        tx.execute(
            "INSERT INTO sample_ages(sample_code,age_model_version,young,old) VALUES(?1,?2,?3,?4)",
            params![s.code, AGE_MODELS[1].0, s.v2.0, s.v2.1],
        )?;
        for (ti, c) in s.counts.iter().enumerate() {
            tx.execute(
                "INSERT INTO counts(sample_code,taxon_code,count) VALUES(?1,?2,?3)",
                params![s.code, TAXA[ti].code, c],
            )?;
        }
    }
    for (top, bottom, note) in MIXED {
        tx.execute(
            "INSERT INTO mixed_layers(depth_top_cm,depth_bottom_cm,note) VALUES(?1,?2,?3)",
            params![top, bottom, note],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 清空并重新导入固定 fixture（验收复核用）。
pub fn reset(conn: &mut Connection) -> rusqlite::Result<()> {
    wipe(conn)?;
    seed(conn)
}
