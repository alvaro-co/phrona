"use strict";

const $ = (id) => document.getElementById(id);
const qInput = $("q");
const state = {
  category: "web",
  engines: new Set(),
  allEngines: {},
  busy: false,
};

document.documentElement.dataset.theme =
  localStorage.getItem("meta-theme") || "light";
$("theme-btn").textContent = document.documentElement.dataset.theme === "dark" ? "\u263e" : "\u263d";
$("theme-btn").addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("meta-theme", next);
  $("theme-btn").textContent = next === "dark" ? "\u263e" : "\u263d";
});

/* ---------- engines ---------- */
async function loadEngines() {
  try {
    const r = await fetch("/v1/engines");
    state.allEngines = await r.json();
    renderEngines();
  } catch (e) {
    console.warn("engines unavailable:", e);
  }
}

function renderEngines() {
  const row = $("engines-row");
  row.textContent = "";
  const list = state.allEngines[state.category] || [];
  for (const name of list) {
    const chip = document.createElement("button");
    chip.className = "chip" + (state.engines.has(name) ? " active" : "");
    chip.textContent = name;
    chip.addEventListener("click", () => {
      if (state.engines.has(name)) state.engines.delete(name);
      else state.engines.add(name);
      chip.classList.toggle("active");
    });
    row.appendChild(chip);
  }
}

/* ---------- categories ---------- */
for (const chip of document.querySelectorAll("#category-row .chip")) {
  chip.addEventListener("click", () => {
    document.querySelectorAll("#category-row .chip").forEach((c) => c.classList.remove("active"));
    chip.classList.add("active");
    state.category = chip.dataset.cat;
    state.engines.clear();
    renderEngines();
  });
}
document.querySelector("#category-row .chip[data-cat=web]").classList.add("active");

/* ---------- suggestions ---------- */
let suggestTimer = null;
qInput.addEventListener("input", () => {
  $("clear-btn").hidden = qInput.value.length === 0;
  clearTimeout(suggestTimer);
  if (qInput.value.trim().length < 2) {
    $("suggestions").hidden = true;
    return;
  }
  suggestTimer = setTimeout(fetchSuggestions, 180);
});

async function fetchSuggestions() {
  const q = qInput.value.trim();
  try {
    const r = await fetch(`/v1/suggest?q=${encodeURIComponent(q)}`);
    const d = await r.json();
    const box = $("suggestions");
    box.textContent = "";
    const seen = new Set();
    let n = 0;
    for (const [, list] of Object.entries(d.suggestions || {})) {
      for (const s of list) {
        if (seen.has(s) || n >= 8) continue;
        seen.add(s); n++;
        const chip = document.createElement("button");
        chip.className = "chip";
        chip.textContent = s;
        chip.addEventListener("mousedown", (e) => e.preventDefault());
        chip.addEventListener("click", () => {
          qInput.value = s;
          box.hidden = true;
          doSearch();
        });
        box.appendChild(chip);
      }
    }
    box.hidden = n === 0;
  } catch {
    /* ignore suggestion errors */
  }
}

/* ---------- search ---------- */
$("search-btn").addEventListener("click", doSearch);
qInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") doSearch();
});
$("clear-btn").addEventListener("click", () => {
  qInput.value = "";
  $("clear-btn").hidden = true;
  qInput.focus();
});

async function doSearch() {
  const q = qInput.value.trim();
  if (!q || state.busy) return;
  $("suggestions").hidden = true;
  state.busy = true;
  const resultsEl = $("results");
  resultsEl.innerHTML = "";
  $("answer").hidden = true;
  $("meta").hidden = true;
  $("empty").hidden = true;
  const spinner = document.createElement("div");
  spinner.className = "spinner";
  resultsEl.appendChild(spinner);

  const params = new URLSearchParams({
    q,
    category: state.category,
    max_results: $("max-results").value || "20",
  });
  if (state.engines.size) params.set("engines", [...state.engines].join(","));
  const region = $("region").value.trim();
  if (region) params.set("region", region);
  const tr = $("time-range").value;
  if (tr) params.set("time_range", tr);

  try {
    const r = await fetch(`/v1/search?${params}`);
    const d = await r.json();
    if (!r.ok) throw new Error(d.error || `HTTP ${r.status}`);
    renderMeta(d);
    if (d.answer) renderAnswer(d.answer);
    renderResults(d.results, d.total);
    if (!d.results.length) $("empty").hidden = false;
  } catch (e) {
    const el = document.createElement("div");
    el.className = "error-banner";
    el.textContent = "Search failed: " + e.message;
    resultsEl.textContent = "";
    resultsEl.appendChild(el);
  } finally {
    state.busy = false;
  }
}

function renderMeta(d) {
  const ok = (d.engines || []).filter((e) => e.status === "ok" && e.results > 0);
  $("meta").textContent = `${d.total} results in ${d.elapsed_ms} ms · engines: ${ok.map((e) => e.name).join(", ") || "—"}`;
  $("meta").hidden = false;
}

function renderAnswer(a) {
  const el = $("answer");
  el.textContent = a;
  el.hidden = false;
}

function esc(s) {
  const div = document.createElement("div");
  div.textContent = s ?? "";
  return div.innerHTML;
}

function renderResults(results, total) {
  const el = $("results");
  el.className = "results" + (state.category === "images" ? " images" : "");
  el.textContent = "";
  for (const r of results) el.appendChild(card(r));
  if (state.category === "videos") {
    el.querySelectorAll(".vid-thumb img").forEach((img) => {
      img.onerror = () => { img.parentElement.remove(); };
    });
  }
}

function card(r) {
  const wrap = document.createElement("article");
  const sources = (r.engines || []).map((e) => `<span class="source-chip">${esc(e)}</span>`).join("");
  if (state.category === "images") {
    wrap.className = "image-card";
    wrap.innerHTML = `
      <img loading="lazy" src="${esc(r.image_url || r.thumbnail_url)}" alt="${esc(r.title)}" onerror="this.style.visibility='hidden'">
      <div class="cap"><span class="t">${esc(r.title)}</span><span class="d">${r.width ? esc(`${r.width}×${r.height}`) : ""}</span></div>`;
    wrap.addEventListener("click", (e) => { e.preventDefault(); window.open(r.url, "_blank"); });
    wrap.href = r.url;
    return wrap;
  }
  wrap.className = "card";
  const sub = [];
  if (r.published) sub.push(`<span>${esc(r.published)}</span>`);
  if (r.source) sub.push(`<span>${esc(r.source)}</span>`);
  if (r.author) sub.push(`<span>${esc(r.author)}</span>`);
  if (r.publisher) sub.push(`<span>${esc(r.publisher)}</span>`);
  if (r.duration) sub.push(`<span class="vid-dur">${esc(r.duration)}</span>`);
  if (r.views) sub.push(`<span>${Number(r.views).toLocaleString()} views</span>`);
  if (r.uploader) sub.push(`<span>${esc(r.uploader)}</span>`);
  const thumb = state.category === "videos" && r.thumbnail_url
    ? `<div class="vid-thumb"><img loading="lazy" src="${esc(r.thumbnail_url)}" alt=""><span class="vid-dur">${esc(r.duration || "")}</span></div>`
    : "";
  wrap.innerHTML = `
    <h3><a href="${esc(r.url)}" target="_blank" rel="noopener">${esc(r.title)}</a></h3>
    <div class="url">${esc(r.url)}</div>
    ${thumb}
    ${r.description ? `<p class="desc">${esc(r.description)}</p>` : ""}
    ${sub.length ? `<div class="sub">${sub.join("")}</div>` : ""}
    <div class="sources">${sources}</div>`;
  return wrap;
}

loadEngines();
$("empty").hidden = false;
qInput.focus();
