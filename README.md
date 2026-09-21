# 孢粉演替台（Pollen Succession Stage）

湖芯孢粉计数的本地分析台：在 **数量（原计数）／相对比例／Hellinger 方差稳定变换（√p）** 之间切换，
把 **多个相邻样本的组合** 之间的组成转变列为 **生态带边界候选**，给出组间差异、方向、年代区间与支持度。
每次分带都 **固定词表版本与年代模型版本**，运行记录可导出、清空数据库后可重新导入并复算复核。

技术栈：Rust + Axum 0.7 + SQLite（rusqlite，bundled）+ 原生 HTML/JS/SVG，无前端构建、无外部网络依赖。

## 安装与演示

```bash
cargo fetch --locked
cargo test --locked
cargo run --locked -- --listen 127.0.0.1:5572
# 浏览器访问 http://127.0.0.1:5572 ，页面标题为“孢粉演替台”
```

可选参数：

```bash
cargo run --locked -- --listen 127.0.0.1:5572 --db ./pollen-stage.sqlite3
```

首次启动若数据库为空，会自动写入固定 fixture（`src/fixture.rs`，版本 `fixture-v1`）与两个演示运行。

## 数据口径（务必先读）

1. **结构零 vs 缺失计数**
   - `counts.count = 0` 表示 **结构零**（该样本确实数到 0 粒），参与分母总和。
   - `counts.count = NULL` 表示 **缺失计数**（未鉴定/未记录），**不计入分母**，页面显示“缺”。
   - 二者在数据表与比例计算中严格区分。
2. **零总计数不生成全零百分比**
   - 某样本分母总和为 0（如无花粉层 S05）时，恒被排除（原因 `zero_total`），
     其比例字段是 `null` 而不是 0.0，避免出现“全零组成曲线”。
3. **孢粉总和（分母）可选**
   - 默认分母由词表 `default_sum` 决定：陆生孢粉（乔木、灌木、陆生草本）入总和，
     水生（香蒲）与蕨类孢子默认 **不入** 总和。
   - 页面上可勾选进入分母的生态组；改变分母会改变比例与边界 Hellinger 支持度（验收点）。
   - 比例只对分母分类单元计算；非分母单元仍展示原计数。
4. **低计数样本排除**
   - 阈值 `min_sum`（默认 50）。分母总和低于阈值的样本被排除（`low_total(<50)`），
     零总和样本无论阈值如何都排除。排除样本在组合之间形成 **断档（gap）**，边界仍列出但标注断档。
5. **相邻样本组合（block size k）**
   - 纳入样本按深度（浅=新 → 深=老）顺序，每 k 个聚合为一个区块；
     区块内分类单元计数相加（缺失不加，结构零加）；边界候选产生于相邻区块之间，不跨越排除断档。
6. **三种视图**
   - `counts`：原始计数；`proportion`：计数/分母总和；`hellinger`：√p（方差稳定）。
   - 组间差异用两区块在共同“有观测”分母分类单元上的 **Hellinger 距离**
     （√p 欧氏距离 / √2，共同集合重新归一化，避免把缺失当零）。
     支持度：`strong ≥ 0.40`，`medium ≥ 0.20`，否则 `weak`。
7. **混层（再搬运/生物扰动）**
   - fixture 中 140–200 cm 为混层。锚点样本落入混层的边界标记 **imprecise（不精确）**，
     不能作为精确边界“锦标”，但仍显示其指示的 **方向**（哪些类群上/下伏增加）。
8. **年代区间：相交合并，不相交保留多峰**
   - 边界年代取上锚（较新样本）与下锚（较老样本）年代区间的交集：
     交集非空 → 合并为一个年代解释（**取交集区间，绝不平均成年份点**）；
     交集为空（年代倒挂/缺段）→ 保留 **两个候选峰（multi-modal）**，页面分别显示。
   - fixture 关键边界 S03↔S06：
     - `age-v1-linear`：两区间不相交（800–1000 vs 4500–5200）→ **多峰**；
     - `age-v2-bacon`：两区间在 1000–1020 相交 → **合并为单峰**。
9. **固定词表与年代模型**
   - 每次分带记录词表指纹 `vocab-v1-…`（FNV-1a）与 `age_model_version`，随运行结果持久化。

## 页面内容

- ① 分带设置：分母生态组勾选、`min_sum`、组合大小 k、年代模型版本、备注；
- ② 运行记录：历史分带、导出 JSON、清空后导入复核、重置为固定 fixture；
- ③ 结果：数量/比例/√p 三态切换，组成曲线（SVG，含排除层位虚线）、原计数表
  （缺失斜体、结构零红色、排除行变暗）、边界候选卡片（深度、Hellinger、支持度、
  组/类群方向、断档、混层不精确、年代单峰/多峰与来源样本）。

## HTTP API

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 操作页面 |
| GET | `/api/taxa` | 固定词表、生态组、词表指纹 |
| GET | `/api/age-models` | 年代模型版本 |
| GET | `/api/dataset` | 样本/年代/混层/计数（含 NULL 缺失） |
| GET/POST | `/api/runs` | 运行列表 / 新建分带（JSON 请求体） |
| GET | `/api/runs/:id` | 单次分带完整结果 |
| GET | `/api/export` | 导出全部数据与运行（JSON） |
| POST | `/api/import` | **清空**数据库后导入导出包，并对每个运行 **复算比对**（`verified`/`mismatches`） |
| POST | `/api/reset` | 清空并重新写入固定 fixture 与演示运行 |

新建分带请求体示例：

```json
{
  "label": "仅乔木分母复核",
  "denominator_groups": ["乔木"],
  "min_sum": 50,
  "block_size": 2,
  "age_model_version": "age-v1-linear"
}
```

`denominator_groups` 省略时用词表默认（陆生孢粉组）。

## 重放与复核

1. 在页面或 `GET /api/export` 下载 `pollen-stage-export.json`（含原始数据与全部运行配置/结果）。
2. 点击“清空并导入复核…”（或 `POST /api/import`）：服务端先清空，再在事务内写回基础数据，
   随后对每个运行用当前统计代码 **重新计算**，与导出结果逐字段比较
   （`config/vocab_fingerprint/samples/blocks/boundaries/transforms`），
   一致时 `verified=true`，否则在 `mismatches` 中列出差异运行与字段。
3. `POST /api/reset` 可随时回到固定 fixture，便于从零复核。

也可以直接删掉数据库文件后重新 `cargo run`，等价于重置 + 写入演示运行。

## 固定 fixture 概览（Example Lake，12 个样本，10 个分类单元）

- 上部 S01–S03：全新世暖期落叶阔叶林；S02/S03 的香蒲计数为 **缺失 NULL**。
- S04：**低总计数层**（总和 8，默认被 `min_sum=50` 排除）。
- S05：**无花粉层**（陆生结构零，总和 0，不生成比例）。
- 下部 S06–S07：冷期草原-云杉；S08–S09 位于 **140–200 cm 混层**（边界不精确但有方向）。
- S10–S12：干冷/冰期草原，S10 蕨类孢子缺失。
- 两套年代模型覆盖同一关键边界的“多峰 vs 合并”两种解释。

## 测试

```bash
cargo test --locked
```

- `tests/unit_analysis.rs`（7 项）：零总和不出比例、缺失不充分母、多峰不平均、
  Hellinger 边界性质、分母切换改变支持度、混层不精确但保留方向、变换键名。
- `tests/server_api.rs`（8 项）：页面标题、fixture 零/缺失/混层/双模型、
  v1 多峰与 v2 合并、分母组改变支持度、导出→清空→导入复算一致、重置与词表指纹、非法年代模型拒绝。

## 目录

```
Cargo.toml
src/
  main.rs        # CLI 入口（--listen/--db）
  server.rs      # Axum 路由与处理器、演示运行
  analysis.rs    # 变换、分块、Hellinger、方向、年代多峰、词表指纹
  db.rs          # SQLite schema/查询/运行持久化
  fixture.rs     # 固定 fixture 与 reset
  io_export.rs   # 导出、清空导入、复算复核
  models.rs      # 数据模型与固定生态组词表
static/
  index.html     # 操作页面
  app.js         # 前端逻辑与 SVG 曲线
tests/           # 单元 + HTTP 集成测试
```
