"use strict";
// 孢粉演替台前端逻辑：原生 JS，无外部依赖。

const state = {
  taxa: [],
  ecoGroups: [],
  ageModels: [],
  selectedGroups: new Set(),
  runs: [],
  current: null,
  transform: "counts",
};

const PALETTE = [
  "#2f7d4f", "#8a5a2b", "#35698f", "#b45309", "#9d174d",
  "#6b21a8", "#1d4e89", "#0f766e", "#a16207", "#7c2d12",
];

const $ = (id) => document.getElementById(id);

function toast(msg, isErr) {
  const t = $("toast");
  t.textContent = msg;
  t.style.background = isErr ? "#7a2420" : "#22352a";
  t.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove("show"), 3200);
}

async function api(path, opts) {
  const res = await fetch(path, opts || {});
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch (_) { data = text; }
  if (!res.ok) {
    const msg = (data && data.error) ? data.error : ("请求失败 " + res.status);
    throw new Error(msg);
  }
  return data;
}

function fmt(n, digits) {
  if (n === null || n === undefined || (typeof n === "number" && Number.isNaN(n))) return "—";
  if (typeof n !== "number") return String(n);
  return n.toLocaleString("zh-CN", { maximumFractionDigits: digits === undefined ? 3 : digits });
}

function ageText(p) {
  return fmt(p.young, 0) + "–" + fmt(p.old, 0) + " cal a BP";
}

function colorFor(code) {
  const i = state.taxa.findIndex((t) => t.code === code);
  return PALETTE[(i >= 0 ? i : 0) % PALETTE.length];
}

function renderGroupChips() {
  const box = $("groupChips");
  box.innerHTML = "";
  state.ecoGroups.forEach((g) => {
    const chip = document.createElement("span");
    chip.className = "chip" + (state.selectedGroups.has(g) ? " on" : "");
    chip.textContent = g;
    chip.onclick = () => {
      if (state.selectedGroups.has(g)) state.selectedGroups.delete(g);
      else state.selectedGroups.add(g);
      renderGroupChips();
    };
    box.appendChild(chip);
  });
  const n = state.taxa.filter((t) => state.selectedGroups.has(t.eco_group)).length;
  $("denomHint").textContent =
    "当前分母组包含 " + state.selectedGroups.size + " 个生态组 / " + n + " 个分类单元；" +
    "水生与蕨类默认不进入孢粉总和。";
}

function renderAgeModels() {
  const sel = $("ageModel");
  sel.innerHTML = "";
  state.ageModels.forEach((m) => {
    const o = document.createElement("option");
    o.value = m.version;
    o.textContent = m.label + "（" + m.version + "）";
    sel.appendChild(o);
  });
}

async function initPage() {
  try {
    const [tx, am, runs] = await Promise.all([
      api("/api/taxa"),
      api("/api/age-models"),
      api("/api/runs"),
    ]);
    state.taxa = tx.taxa;
    state.ecoGroups = tx.eco_groups;
    $("vocabFp").textContent = tx.vocab_fingerprint;
    state.ageModels = am.age_models;
    state.runs = runs.runs;

    // 默认分母 = 词表 default_sum
    state.selectedGroups = new Set(
      state.taxa.filter((t) => t.default_sum).map((t) => t.eco_group)
    );
    renderGroupChips();
    renderAgeModels();
    renderRunList();

    if (state.runs.length) {
      await selectRun(state.runs[state.runs.length - 1].id);
    }
  } catch (e) {
    toast(e.message, true);
  }
}

function renderRunList() {
  const box = $("runList");
  box.innerHTML = "";
  if (!state.runs.length) {
    box.innerHTML = '<p class="muted small">暂无运行。</p>';
    return;
  }
  state.runs.forEach((r) => {
    const d = document.createElement("div");
    d.className = "run" + (state.current && state.current.run_id === r.id ? " on" : "");
    const c = r.config || {};
    d.innerHTML =
      "<div><b>#" + r.id + "</b> " + escapeHtml(r.label) + "</div>" +
      '<div class="muted small">' + escapeHtml(c.age_model_version || "") +
      " · min_sum=" + (c.min_sum ?? "—") + " · k=" + (c.block_size ?? "—") + "</div>";
    d.onclick = () => selectRun(r.id);
    box.appendChild(d);
  });
}

function escapeHtml(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

async function selectRun(id) {
  try {
    const result = await api("/api/runs/" + id);
    state.current = result;
    renderAll();
    renderRunList();
  } catch (e) { toast(e.message, true); }
}

function renderAll() {
  renderMeta();
  renderLegend();
  renderCurves();
  renderCountTable();
  renderBoundaries();
}

function renderMeta() {
  const r = state.current;
  if (!r) { $("runMeta").textContent = "尚未选择运行。"; return; }
  const c = r.config;
  const excluded = r.samples.filter((s) => !s.included);
  const zeros = r.samples.filter((s) => s.zero_total);
  const multi = r.boundaries.filter((b) => b.multi_modal).length;
  const imp = r.boundaries.filter((b) => b.imprecise).length;
  $("runMeta").innerHTML =
    "<b>#" + r.run_id + " " + escapeHtml(r.label) + "</b> · " +
    "年代模型 <code>" + escapeHtml(c.age_model_version) + "</code> · k=" + c.block_size +
    " · min_sum=" + c.min_sum + "<br>" +
    '<span class="muted">分母生态组：' + escapeHtml((c.denominator_groups || []).join("、")) +
    " ｜ 词表指纹 <code>" + r.vocab_fingerprint + "</code></span><br>" +
    "纳入 " + (r.samples.length - excluded.length) + "/" + r.samples.length +
    " 个样本（排除 " + excluded.length + "，其中全零 " + zeros.length +
    "）；边界候选 " + r.boundaries.length + " 个，多峰 " + multi + " 个，混层不精确 " + imp + " 个。";
}

function renderLegend() {
  const box = $("legend");
  box.innerHTML = "";
  state.taxa.forEach((t) => {
    const s = document.createElement("span");
    s.innerHTML = '<i class="sw" style="background:' + colorFor(t.code) + '"></i>' +
      escapeHtml(t.name) + "（" + escapeHtml(t.code) + "）";
    box.appendChild(s);
  });
}

// 深度行自上而下：浅（新）→ 深（老）。result.samples 已按深度升序，表格逆序绘制。
function renderCountTable() {
  const r = state.current;
  const box = $("countTable");
  if (!r) { box.innerHTML = ""; return; }
  const denomCodes = new Set(r.samples[0]?.taxa.filter((t) => t.denominator).map((t) => t.code) || []);
  let html = "<table><thead><tr><th>样本</th><th>深度</th><th>年代</th><th>总和</th>";
  state.taxa.forEach((t) => { html += "<th title='" + escapeHtml(t.name) + "'>" + escapeHtml(t.code) + "</th>"; });
  html += "</tr></thead><tbody>";

  const rows = r.samples.slice().reverse();
  rows.forEach((s) => {
    html += "<tr class='" + (s.included ? "" : "excluded") + "'>";
    const mix = s.mixed ? ' <span class="tag imp">混层</span>' : "";
    const ex = !s.included ? ' <span class="tag gap">' + escapeHtml(s.exclude_reason || "排除") + "</span>" : "";
    html += "<td>" + escapeHtml(s.code) + mix + ex + "</td>";
    html += "<td>" + fmt(s.depth_cm, 1) + "</td>";
    html += "<td>" + fmt(s.age_young, 0) + "–" + fmt(s.age_old, 0) + "</td>";
    html += "<td><b>" + s.denom_sum + "</b></td>";
    state.taxa.forEach((t) => {
      const cell = s.taxa.find((x) => x.code === t.code);
      if (!cell) { html += '<td class="miss">—</td>'; return; }
      if (cell.count === "missing") {
        html += '<td class="miss" title="缺失计数（区别于结构零）">缺</td>';
      } else if (cell.count === 0 && denomCodes.has(t.code)) {
        html += '<td class="zero">0</td>';
      } else {
        let v = cell.count;
        if (state.transform === "proportion") {
          v = cell.prop === null || cell.prop === undefined ? "—" : (cell.prop * 100).toFixed(1) + "%";
        } else if (state.transform === "hellinger") {
          v = cell.prop === null || cell.prop === undefined ? "—" : Math.sqrt(cell.prop).toFixed(2);
        }
        html += "<td>" + (typeof v === "number" ? v : escapeHtml(v)) + "</td>";
      }
    });
    html += "</tr>";
  });
  html += "</tbody></table>";
  box.innerHTML = html;
}

// 组成曲线：每个分母分类单元一个迷你纵剖面（深度轴向下），曲线宽度＝该变换值。
function renderCurves() {
  const r = state.current;
  const box = $("curves");
  if (!r) { box.innerHTML = ""; return; }

  const included = r.samples; // 升序
  const W = 78, H = 380, pad = 16;
  const denomTaxa = (included[0]?.taxa || []).filter((t) => t.denominator).map((t) => t.code);

  let maxV = 0;
  included.forEach((s) => s.taxa.forEach((t) => {
    if (!t.denominator) return;
    let v = 0;
    if (state.transform === "counts") v = t.count === "missing" ? 0 : t.count;
    else if (state.transform === "proportion") v = t.prop || 0;
    else v = t.prop ? Math.sqrt(t.prop) : 0;
    if (v > maxV) maxV = v;
  }));
  if (state.transform === "proportion") maxV = Math.max(maxV, 1);
  if (state.transform === "hellinger") maxV = Math.max(maxV, 1);
  if (maxV === 0) maxV = 1;

  const depths = included.map((s) => s.depth_cm);
  const dMin = Math.min(...depths), dMax = Math.max(...depths);
  const yOf = (d) => pad + (d - dMin) / Math.max(dMax - dMin, 1) * (H - 2 * pad);

  let svg = '<svg width="' + (denomTaxa.length * (W + 6) + 40) + '" height="' + (H + 54) + '">';
  denomTaxa.forEach((code, ci) => {
    const x0 = 34 + ci * (W + 6);
    const taxon = state.taxa.find((t) => t.code === code);
    svg += '<rect x="' + x0 + '" y="' + pad + '" width="' + W + '" height="' + (H - 2 * pad) +
           '" fill="#fbfdfb" stroke="#e2e9e1"/>';
    // 数据折线（左右对称填充）
    let ptsL = "", ptsR = "";
    included.forEach((s) => {
      const cell = s.taxa.find((t) => t.code === code);
      let v = 0;
      if (state.transform === "counts") v = cell && cell.count !== "missing" ? cell.count : 0;
      else v = cell && cell.prop ? cell.prop : 0;
      if (state.transform === "hellinger") v = v ? Math.sqrt(v) : 0;
      const w = (v / maxV) * (W / 2 - 3);
      const y = yOf(s.depth_cm);
      ptsL += (x0 + W / 2 - w).toFixed(1) + "," + y.toFixed(1) + " ";
      ptsR += (x0 + W / 2 + w).toFixed(1) + "," + y.toFixed(1) + " ";
    });
    const col = colorFor(code);
    svg += '<polygon points="' + ptsL + ptsR.trim().split(" ").reverse().join(" ") +
           '" fill="' + col + '" fill-opacity="0.18" stroke="' + col + '" stroke-width="1.3"/>';
    // 混层带
    r.samples.forEach(() => {});
    // 排除层位虚线
    included.filter((s) => !s.included).forEach((s) => {
      const y = yOf(s.depth_cm);
      svg += '<line x1="' + x0 + '" y1="' + y + '" x2="' + (x0 + W) + '" y2="' + y +
             '" stroke="#b45309" stroke-dasharray="3 3"/>';
    });
    svg += '<text x="' + (x0 + W / 2) + '" y="' + (H + 14) + '" font-size="10" text-anchor="middle">' +
           escapeHtml(taxon ? taxon.code : code) + "</text>";
    svg += '<text x="' + (x0 + W / 2) + '" y="' + (H + 28) + '" font-size="9" fill="#5c6b61" text-anchor="middle">' +
           escapeHtml(taxon ? taxon.eco_group : "") + "</text>";
  });
  // 深度轴
  svg += '<text x="6" y="' + pad + '" font-size="10" fill="#5c6b61">' + fmt(dMin, 0) + ' cm(新)</text>';
  svg += '<text x="6" y="' + (H - pad + 4) + '" font-size="10" fill="#5c6b61">' + fmt(dMax, 0) + ' cm(老)</text>';
  svg += '<text x="6" y="12" font-size="9" fill="#8a5a2b">橙虚线=排除层位</text>';
  svg += "</svg>";
  box.innerHTML = svg;
}

function renderBoundaries() {
  const r = state.current;
  const box = $("boundaryList");
  if (!r) { box.innerHTML = '<p class="muted">尚无边界候选。</p>'; return; }
  if (!r.boundaries.length) { box.innerHTML = '<p class="muted">无相邻区块（可能全部样本被排除）。</p>'; return; }

  box.innerHTML = r.boundaries.map((b, idx) => {
    const tags =
      '<span class="tag ' + b.support + '">支持度:' +
      ({ strong: "强", medium: "中", weak: "弱" }[b.support] || b.support) +
      " H=" + fmt(b.hellinger) + "</span>" +
      (b.imprecise ? '<span class="tag imp">混层·边界不精确</span>' : "") +
      (b.multi_modal ? '<span class="tag multi">年代多峰·不可平均</span>' : "") +
      (b.gap ? '<span class="tag gap">跨排除层断档</span>' : "");

    const peaks = (b.age_peaks || []).map((p, i) =>
      '<span class="peak' + (b.multi_modal ? " multi" : "") + '">' +
      (b.multi_modal ? "峰" + (i + 1) + "：" : "") + ageText(p) +
      ' <span class="muted">[' + p.source_samples.join(",") + "]</span></span>").join("");

    const groupRows = (b.group_changes || []).filter((g) => Math.abs(g.delta_prop) >= 0.01).map((g) => {
      const cls = g.delta_prop > 0 ? "delta-up" : "delta-down";
      return "<span class='" + cls + "'>" + escapeHtml(g.eco_group) + " " +
        (g.delta_prop > 0 ? "▲" : "▼") + (g.delta_prop * 100).toFixed(1) + "%</span>";
    }).join(" &nbsp; ");

    const topTaxa = (b.taxon_changes_top || []).slice(0, 4).map((t) =>
      escapeHtml(t.name) + "(" + (t.delta_prop * 100).toFixed(1) + "pp)").join("，");

    return '<div class="bcard' + (b.imprecise ? " imp" : "") + '">' +
      "<div><b>候选 #" + (idx + 1) + " · " + b.boundary_id + "</b> · 深度约 " +
      fmt(b.depth_cm, 1) + " cm &nbsp; " + tags + "</div>" +
      '<div class="muted small">上覆区块：' + b.upper_samples.join(",") +
      " ｜ 下伏区块：" + b.lower_samples.join(",") + "</div>" +
      "<div><span class='muted small'>方向：</span>" + escapeHtml(b.direction) +
      ' <span class="small">' + groupRows + "</span></div>" +
      '<div class="small muted">主要分类单元变化：' + topTaxa + "</div>" +
      '<div class="peaks"><span class="muted small">年代(' + escapeHtml(b.age_model_version) + ')：</span>' +
      peaks + "</div>" +
      (b.mixed_notes && b.mixed_notes.length
        ? '<div class="small" style="color:#6b21a8">' + b.mixed_notes.map(escapeHtml).join("；") + "</div>"
        : "") +
      "</div>";
  }).join("");
}

// ---------- 事件 ----------

document.querySelectorAll(".seg button").forEach((btn) => {
  btn.onclick = () => {
    document.querySelectorAll(".seg button").forEach((x) => x.classList.remove("on"));
    btn.classList.add("on");
    state.transform = btn.dataset.t;
    if (state.current) { renderCurves(); renderCountTable(); }
  };
});

$("runBtn").onclick = async () => {
  if (!state.selectedGroups.size) { toast("请至少选择一个分母生态组", true); return; }
  const body = {
    label: $("runLabel").value.trim() || null,
    denominator_groups: Array.from(state.selectedGroups),
    min_sum: parseInt($("minSum").value, 10) || 0,
    block_size: parseInt($("blockSize").value, 10) || 1,
    age_model_version: $("ageModel").value,
  };
  try {
    const created = await api("/api/runs", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    $("runLabel").value = "";
    const runs = await api("/api/runs");
    state.runs = runs.runs;
    await selectRun(created.id);
    toast("分带完成：#" + created.id);
  } catch (e) { toast(e.message, true); }
};

$("exportBtn").onclick = async () => {
  try {
    const data = await api("/api/export");
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "pollen-stage-export.json";
    a.click();
    URL.revokeObjectURL(a.href);
  } catch (e) { toast(e.message, true); }
};

$("importBtn").onclick = () => $("importFile").click();
$("importFile").onchange = async (ev) => {
  const f = ev.target.files[0];
  if (!f) return;
  try {
    const text = await f.text();
    const data = JSON.parse(text);
    const rep = await api("/api/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    });
    $("ioMsg").textContent =
      "导入 " + rep.imported_runs + " 个运行，复算 " + rep.replayed_runs +
      (rep.verified ? "，全部一致 ✓" : ("，不一致 " + rep.mismatches.length + " 项"));
    toast(rep.verified ? "导入并复算复核通过" : "复算存在不一致", !rep.verified);
    await refreshAfterMutation();
  } catch (e) { toast(e.message, true); }
  ev.target.value = "";
};

$("resetBtn").onclick = async () => {
  if (!confirm("将清空数据库并重新导入固定 fixture，继续？")) return;
  try {
    const rep = await api("/api/reset", { method: "POST" });
    $("ioMsg").textContent = "已重置为 " + rep.fixture + "，预置 " + rep.seeded_demo_runs.length + " 个演示运行。";
    toast("数据库已重置并重导 fixture");
    await refreshAfterMutation();
  } catch (e) { toast(e.message, true); }
};

async function refreshAfterMutation() {
  const runs = await api("/api/runs");
  state.runs = runs.runs;
  renderRunList();
  if (state.runs.length) await selectRun(state.runs[state.runs.length - 1].id);
  else { state.current = null; renderAll(); renderRunList(); }
}

initPage();
