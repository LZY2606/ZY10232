"use strict";

const state = { fixture: null, result: null, transform: "proportion", ageModels: [] };
const TAX_COLORS = ["#2f6f4e", "#5a9e6f", "#8fbc8f", "#c19a3d", "#b5651d", "#6b7a8f", "#8e5a9e", "#c44e52", "#937860"];

const $ = (id) => document.getElementById(id);

async function api(path, opts) {
  const res = await fetch(path, opts);
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch (_) { data = { raw: text }; }
  if (!res.ok) throw new Error((data && data.error) || res.statusText);
  return data;
}

function setStatus(msg, kind) {
  const el = $("run-status");
  el.textContent = msg || "";
  el.className = "status " + (kind || "");
}

async function loadFixture() {
  const data = await api("/api/fixture");
  state.fixture = data.fixture;
  state.fixtureHash = data.fixture_hash;
  const fx = state.fixture;
  state.ageModels = Array.from(new Set(fx.samples.flatMap((s) => Object.keys(s.ages || {})))).sort();
  renderMeta();
  renderControls();
}

function renderMeta() {
  const fx = state.fixture;
  $("site-meta").innerHTML =
    `<span>站点 <b>${esc(fx.site)}</b></span>` +
    `<span>固定柱样指纹 <b>${esc(state.fixtureHash)}</b></span>` +
    `<span>年代单位 <b>${esc(fx.age_unit)}</b></span>` +
    `<span>样本 ${fx.samples.length} · 分类单元 ${fx.taxa.length}</span>`;
}

function renderControls() {
  const fx = state.fixture;
  const g = $("group-select");
  g.innerHTML = "";
  for (const [name, taxa] of Object.entries(fx.groups)) {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = `${name}（${taxa.length} 类）`;
    if (name === "all") o.selected = true;
    g.appendChild(o);
  }
  const am = $("age-model");
  am.innerHTML = "";
  for (const m of state.ageModels) {
    const o = document.createElement("option");
    o.value = m;
    o.textContent = m;
    am.appendChild(o);
  }
  if (state.ageModels.includes("am_2024")) am.value = "am_2024";
}

function readInput() {
  const custom = $("denominator-input").value.trim();
  const body = {
    group: $("group-select").value,
    min_total: Number($("min-total").value || 0),
    block_size: Number($("block-size").value || 1),
    age_model: $("age-model").value,
    transform: $("transform").value,
  };
  if (custom) {
    body.denominator_taxa = custom.split(/[,，\s]+/).filter(Boolean);
    body.group = null;
  }
  return body;
}

async function doRun() {
  const body = readInput();
  state.transform = body.transform;
  setStatus("计算中…");
  try {
    const data = await api("/api/runs", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    state.result = data.result;
    state.lastId = data.id;
    renderResult();
    await loadRuns();
    setStatus(`已保存为运行 #${data.id}：${data.result.run_label}`, "ok");
  } catch (e) {
    setStatus("运行失败：" + e.message, "err");
  }
}

function esc(v) {
  return String(v == null ? "" : v).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

function renderResult() {
  const r = state.result;
  $("result-panel").hidden = false;
  $("run-info").innerHTML =
    `<span>词表指纹 <b>${esc(r.vocabulary_hash)}</b></span>` +
    `<span>柱样指纹 <b>${esc(r.fixture_hash)}</b></span>` +
    `<span>年代模型 <b>${esc(r.age_model)}</b></span>` +
    `<span>分母 <b>${r.denominator_taxa.map(esc).join(", ")}</b></span>` +
    `<span>低计数阈值 ≥ <b>${r.min_total}</b></span>` +
    `<span>块大小 <b>k=${r.block_size}</b></span>` +
    `<span>排除样本 <b>${r.excluded_samples.map(esc).join(", ") || "无"}</b></span>`;
  renderBoundaries();
  renderChart();
  renderSampleTable();
}

function fmtRange(p, unit) {
  return `${Math.round(p[0])}–${Math.round(p[1])} ${unit}`;
}

function renderBoundaries() {
  const r = state.result;
  const host = $("boundaries");
  if (!r.boundaries.length) {
    host.innerHTML = `<p class="detail">纳入样本不足，暂无边界候选。</p>`;
    return;
  }
  host.innerHTML = r.boundaries
    .map((b) => {
      const exactTag = b.exact
        ? `<span class="tag exact">精确候选</span>`
        : `<span class="tag fuzzy">混层·仅可能方向</span>`;
      const multiTag = b.multi_peak ? `<span class="tag multi">年代多峰·不平均</span>` : "";
      const peaks = b.age_peaks.map((p) => fmtRange(p, r.age_unit)).join(" &nbsp;｜&nbsp; ");
      const dir = b.possible_direction
        ? `<div class="dir">可能支持方向：<b>${esc(b.possible_direction.taxon)}</b> ${esc(b.possible_direction.sense_cn)}（比例差 ${(b.possible_direction.delta_proportion * 100).toFixed(1)} 个百分点）</div>`
        : `<div class="dir">无明显方向（组成为常量或零）</div>`;
      const contrib = b.contributions
        .filter((c) => Math.abs(c.delta_proportion) > 0)
        .slice(0, 5)
        .map((c) => `${esc(c.taxon)} ${c.delta_proportion >= 0 ? "↑" : "↓"}${(Math.abs(c.delta_proportion) * 100).toFixed(1)}%`)
        .join("；");
      return `<div class="boundary-card ${b.exact ? "exact" : "fuzzy"}">
        <div class="boundary-head">
          <span class="rank">#${b.rank} ${esc(b.id)}</span>
          <span>${esc(b.between.join(" ↔ "))}（深度 ${b.depth_range_cm[0].toFixed(1)}–${b.depth_range_cm[1].toFixed(1)} cm）</span>
          ${exactTag}${multiTag}
          <span class="support">支持度·${esc(labelFor(r.transform))}：<b>${b.support.toFixed(4)}</b>
            （数量 ${b.support_counts.toFixed(3)} / 比例 ${b.support_proportion.toFixed(3)} / 反正弦 ${b.support_angular.toFixed(3)}）</span>
        </div>
        ${dir}
        <div class="detail">年代区间（${esc(r.age_model)}）：${peaks}${b.peak_note ? "；" + esc(b.peak_note) : ""}</div>
        ${b.skipped_samples.length ? `<div class="detail">间隙跨越被排除样本：${b.skipped_samples.map(esc).join(", ")}</div>` : ""}
        ${b.fuzzy_reason ? `<div class="detail">${esc(b.fuzzy_reason)}</div>` : ""}
        <div class="contrib">组成贡献（比例百分点，上盘相对下盘）：${contrib || "—"}</div>
      </div>`;
    })
    .join("");
}

function labelFor(t) {
  return { counts: "数量", proportion: "比例", angular: "反正弦" }[t] || t;
}

function colorFor(code) {
  const codes = state.fixture.taxa.map((t) => t.code);
  const idx = codes.indexOf(code);
  return TAX_COLORS[idx % TAX_COLORS.length];
}

function renderChart() {
  const r = state.result;
  const taxa = r.denominator_taxa;
  const W = 760, rowH = 34, padL = 60, padR = 220, padT = 14, padB = 14;
  const H = padT + padB + r.samples.length * rowH;
  const x0 = padL, xMax = W - padR;
  const depths = r.samples.map((s) => s.depth_cm);
  const dMin = Math.min(...depths), dMax = Math.max(...depths);
  const y = (d) => padT + ((d - dMin) / (dMax - dMin || 1)) * (H - padT - padB);

  let svg = `<svg viewBox="0 0 ${W} ${H}" width="100%" role="img" aria-label="组成曲线">`;
  // 横轴刻度（百分比 / 角度 / 数量占位仅按比例与反正弦 0..1；数量另在表中呈现）
  for (let i = 0; i <= 4; i++) {
    const xv = x0 + ((xMax - x0) * i) / 4;
    const lab = r.transform === "angular" ? (i * 0.5).toFixed(1) + " rad" : (i * 25) + "%";
    svg += `<line x1="${xv}" y1="${padT}" x2="${xv}" y2="${H - padB}" stroke="#eef2ec" />`;
    svg += `<text class="axis" x="${xv}" y="${H - 2}" text-anchor="middle">${lab}</text>`;
  }

  // 样本横线（混层橙色虚线，排除灰色虚线）
  for (const s of r.samples) {
    const yv = y(s.depth_cm);
    let cls = "";
    if (!s.included) cls = "excluded-row";
    else if (s.mixed) cls = "mixed-row";
    svg += `<line class="${cls}" x1="${x0}" y1="${yv}" x2="${xMax}" y2="${yv}" />`;
    svg += `<text class="depth-label" x="${x0 - 6}" y="${yv + 3}" text-anchor="end">${esc(s.code)} ${s.depth_cm.toFixed(1)}cm</text>`;
    if (!s.included) svg += `<text class="axis" x="${xMax + 6}" y="${yv + 3}">排除：${esc((s.excluded_reason || "").split("：")[0])}</text>`;
    if (s.mixed) svg += `<text class="axis" x="${xMax + 6}" y="${yv - 6}" fill="#9a5a13">混层</text>`;
  }

  // 边界线（取间隙深度中点）
  for (const b of r.boundaries.slice(0, 8)) {
    const yv = y((b.depth_range_cm[0] + b.depth_range_cm[1]) / 2);
    svg += `<line class="boundary-line ${b.exact ? "" : "fuzzy"}" x1="${x0}" y1="${yv}" x2="${xMax}" y2="${yv}" />`;
    svg += `<text x="${xMax + 6}" y="${yv + 3}" font-size="10" fill="${b.exact ? "#8a2c2c" : "#9a5a13"}">${esc(b.id)} ${b.support.toFixed(2)}${b.multi_peak ? " ⧉多峰" : ""}</text>`;
  }

  // 各分类单元曲线：按当前视图取值；数量口径曲线用平方根缩放只是示意，精确数值见表。
  const seriesVals = (s, code) => {
    if (r.transform === "proportion") return s.proportions ? s.proportions[code] ?? null : null;
    if (r.transform === "angular") return s.angular ? s.angular[code] != null ? s.angular[code] / (Math.PI / 2) : null : null;
    return null;
  };

  for (const code of taxa) {
    let path = "";
    let started = false;
    for (const s of r.samples) {
      let v = seriesVals(s, code);
      if (v == null) { started = false; continue; }
      const xv = x0 + Math.max(0, Math.min(1, v)) * (xMax - x0);
      const yv = y(s.depth_cm);
      path += `${started ? "L" : "M"}${xv.toFixed(1)},${yv.toFixed(1)} `;
      started = true;
    }
    svg += `<path d="${path}" fill="none" stroke="${colorFor(code)}" stroke-width="1.8" />`;
  }
  svg += `</svg>`;

  // 图例
  $("legend").innerHTML =
    taxa.map((c) => `<span><span class="swatch" style="background:${colorFor(c)}"></span>${esc(c)}</span>`).join("") +
    `<span>— 红线=精确边界候选；橙虚线=混层模糊候选；⧉=年代多峰</span>` +
    (r.transform === "counts"
      ? `<span>数量口径以表格与支持度为准（曲线切换到比例/反正弦查看）</span>`
      : "");
  $("chart").innerHTML = svg;
}

function renderSampleTable() {
  const r = state.result;
  const fx = state.fixture;
  const taxa = fx.taxa.map((t) => t.code);
  const countOf = (sampleCode, code) => {
    const row = fx.counts.find((c) => c.sample === sampleCode);
    if (!row || !(code in row) || row[code] === null) return null;
    return row[code];
  };
  const head = `<thead><tr>
    <th>样本</th><th>深度 cm</th><th>混层</th><th>观察总计数</th><th>分母总计数</th>
    ${taxa.map((t) => `<th>${esc(t)}</th>`).join("")}
    <th>状态</th></tr></thead>`;
  const body = r.samples
    .map((s) => {
      const cells = taxa
        .map((code) => {
          const raw = countOf(s.code, code);
          const inDenom = r.denominator_taxa.includes(code);
          const pct = s.percents && inDenom ? s.percents[code] : null;
          if (raw === null) return `<td class="missing">缺失</td>`;
          let cls = raw === 0 ? "zero" : "";
          let inner = `${raw}`;
          if (pct != null && s.included) {
            inner = `<div class="bar-cell"><span>${raw} · ${pct.toFixed(1)}%</span><div class="bar" style="width:${Math.min(100, pct).toFixed(1)}%"></div></div>`;
          }
          return `<td class="${cls}">${inner}${inDenom ? "" : '<span class="detail">（非分母）</span>'}</td>`;
        })
        .join("");
      const reason = s.included ? "纳入" : esc(s.excluded_reason || "排除");
      return `<tr class="${s.included ? "" : "excluded"}">
        <td>${esc(s.code)}</td><td>${s.depth_cm.toFixed(1)}</td>
        <td>${s.mixed ? "是" : "否"}</td><td>${s.observed_total}</td><td>${s.denominator_total}</td>
        ${cells}<td>${reason}</td></tr>`;
    })
    .join("");
  $("sample-table").innerHTML = head + `<tbody>${body}</tbody>`;
}

async function loadRuns() {
  const data = await api("/api/runs");
  const rows = data.runs || [];
  const head = `<thead><tr>
    <th>#</th><th>标签</th><th>口径</th><th>年代模型</th><th>阈值</th><th>k</th>
    <th>词表指纹</th><th>柱样指纹</th><th>时间 (UTC)</th><th>操作</th></tr></thead>`;
  const body = rows
    .map(
      (x) => `<tr>
      <td>${x.id}</td><td>${esc(x.run_label)}</td><td>${esc(x.transform)}</td><td>${esc(x.age_model)}</td>
      <td>${x.min_total}</td><td>${x.block_size}</td>
      <td>${esc(x.vocabulary_hash)}</td><td>${esc(x.fixture_hash)}</td><td>${esc(x.created_at)}</td>
      <td>
        <a href="/api/runs/${x.id}/export">导出</a> ·
        <a href="#" data-load="${x.id}">查看</a> ·
        <a href="#" data-del="${x.id}">删除</a>
      </td></tr>`
    )
    .join("");
  $("runs-table").innerHTML = head + `<tbody>${body || '<tr><td colspan="10">暂无运行记录</td></tr>'}</tbody>`;
  $("runs-table").querySelectorAll("[data-del]").forEach((a) =>
    a.addEventListener("click", async (e) => {
      e.preventDefault();
      await api(`/api/runs/${a.dataset.del}`, { method: "DELETE" });
      await loadRuns();
    })
  );
  $("runs-table").querySelectorAll("[data-load]").forEach((a) =>
    a.addEventListener("click", async (e) => {
      e.preventDefault();
      const d = await api(`/api/runs/${a.dataset.load}`);
      state.result = d.result;
      state.transform = d.result.transform;
      $("transform").value = d.result.transform;
      renderResult();
      window.scrollTo({ top: 0, behavior: "smooth" });
    })
  );
}

async function replayFile(file) {
  const text = await file.text();
  let payload;
  try { payload = JSON.parse(text); } catch (e) { setStatus("文件不是合法 JSON", "err"); return; }
  try {
    const d = await api("/api/replay", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    const failed = d.report.filter((x) => !x.ok);
    $("replay-report").innerHTML =
      `<p class="status ${failed.length ? "err" : "ok"}">重放复核：${d.imported}/${d.checked} 条一致并入库。` +
      (failed.length ? " 不一致条目：" + failed.map((f) => `#${f.index}(${esc(f.reason)})`).join("；") : "") +
      `</p>`;
    await loadRuns();
  } catch (e) {
    setStatus("重放失败：" + e.message, "err");
  }
}

async function resetScope(scope) {
  const d = await api("/api/admin/reset", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ scope }),
  });
  setStatus(scope === "all" ? `已清空并重导固定柱样 ${d.site}（${d.fixture_hash}）` : "已清空运行记录", "ok");
  await loadRuns();
}

function wire() {
  $("run-btn").addEventListener("click", doRun);
  $("export-all-btn").addEventListener("click", () => { window.location.href = "/api/export"; });
  $("reset-runs-btn").addEventListener("click", () => resetScope("runs"));
  $("reset-all-btn").addEventListener("click", () => {
    if (confirm("将清空全部运行并把固定柱样重新导入 SQLite（数据库文件保留）。继续？")) resetScope("all");
  });
  $("replay-file").addEventListener("change", (e) => {
    const f = e.target.files[0];
    if (f) replayFile(f);
    e.target.value = "";
  });
}

(async function init() {
  wire();
  await loadFixture();
  await loadRuns();
  doRun();
})();
