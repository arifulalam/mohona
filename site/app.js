(() => {
  "use strict";
  const REPO = "arifulalam/mohona";
  // Languages --------------------------------------------------------------
  const LANGS = [
    ["en", "English"], ["bn", "বাংলা"], ["hi", "हिन्दी"], ["ur", "اردو"], ["ar", "العربية"], ["zh", "中文"], ["ja", "日本語"],
    ["ru", "Русский"], ["de", "Deutsch"], ["fr", "Français"], ["es", "Español"], ["pt", "Português"], ["tr", "Türkçe"], ["id", "Indonesia"],
  ];
  const RTL = new Set(["ar", "ur"]);
  // Where the visitor is, as far as a page can tell without asking: the time zone names a place.
  const ZONES = {
    bn: [/^Asia\/(Dhaka)$/], hi: [/^Asia\/(Kolkata|Calcutta)$/], ur: [/^Asia\/Karachi$/],
    ar: [/^(Asia\/(Riyadh|Dubai|Baghdad|Kuwait|Qatar|Muscat|Bahrain|Amman|Damascus|Beirut|Aden)|Africa\/(Cairo|Algiers|Tunis|Casablanca|Tripoli|Khartoum))$/],
    zh: [/^Asia\/(Shanghai|Chongqing|Harbin|Urumqi|Hong_Kong|Macau|Taipei)$/], ja: [/^Asia\/Tokyo$/],
    ru: [/^Europe\/(Moscow|Kaliningrad|Samara|Volgograd|Minsk)$/, /^Asia\/(Yekaterinburg|Novosibirsk|Omsk|Krasnoyarsk|Irkutsk|Yakutsk|Vladivostok|Magadan|Kamchatka)$/],
    de: [/^Europe\/(Berlin|Vienna|Zurich|Busingen)$/], fr: [/^Europe\/(Paris|Brussels|Luxembourg|Monaco)$/, /^Africa\/(Dakar|Abidjan)$/],
    es: [/^Europe\/Madrid$/, /^America\/(Mexico_City|Bogota|Lima|Santiago|Buenos_Aires|Argentina\/.*|Caracas|Montevideo|Guayaquil|La_Paz|Asuncion)$/],
    pt: [/^Europe\/Lisbon$/, /^America\/(Sao_Paulo|Bahia|Fortaleza|Recife|Manaus|Belem)$/], tr: [/^Europe\/Istanbul$/],
    id: [/^Asia\/(Jakarta|Makassar|Jayapura|Pontianak)$/],
  };
  const LKEY = "mohona.site.lang";
  const lstore = {
    get() { try { return localStorage.getItem(LKEY); } catch { return null; } },
    set(v) { try { localStorage.setItem(LKEY, v); } catch { /* private mode */ } },
  };
  const have = (c) => LANGS.some(([id]) => id === c);
  function detect() {
    const asked = new URLSearchParams(location.search).get("lang");
    if (asked && have(asked)) return asked;
    const saved = lstore.get();
    if (saved && have(saved)) return saved;
    let zone = "";
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ""; } catch { /* old browser */ }
    for (const [code, rules] of Object.entries(ZONES)) if (rules.some((re) => re.test(zone))) return code;
    for (const l of navigator.languages || [navigator.language || "en"]) {
      const c = String(l).toLowerCase().split("-")[0];
      if (have(c)) return c;
    }
    return "en";
  }
  let lang = "en";
  let dict = {}, base = {};
  const t = (k, vars) => {
    let v = dict[k] ?? base[k] ?? k;
    if (vars) for (const [n, x] of Object.entries(vars)) v = v.replace(`{${n}}`, x);
    return v;
  };
  const load = (c) => fetch(`i18n/${c}.json`).then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))));
  const rerender = [];
  function paint() {
    document.querySelectorAll("[data-i18n]").forEach((el) => {
      const k = el.dataset.i18n;
      if (el.dataset.en === undefined) el.dataset.en = el.innerHTML;
      const v = dict[k];
      el.innerHTML = v !== undefined ? v : el.dataset.en;
    });
    const native = dict._native || "মোহনা";
    document.querySelectorAll(".native").forEach((el) => { el.textContent = lang === "en" ? "মোহনা" : native; });
    document.documentElement.lang = lang;
    document.documentElement.dir = RTL.has(lang) ? "rtl" : "ltr";
    document.title = t("meta_title");
    document.getElementById("meta-desc").setAttribute("content", t("meta_desc"));
    rerender.forEach((f) => f());
  }
  async function setLang(c, remember) {
    if (!have(c)) c = "en";
    try { dict = c === "en" ? {} : await load(c); } catch { dict = {}; c = "en"; }
    lang = c;
    if (remember) lstore.set(c);
    langSel.value = c;
    paint();
  }
  const langSel = document.getElementById("lang");
  for (const [id, label] of LANGS) langSel.add(new Option(label, id));
  langSel.addEventListener("change", () => setLang(langSel.value, true));

  const THEMES = [
    ["system", "System", "#1565c0"], ["light", "Light", "#f6f8fb"], ["dark", "Dark", "#14171c"],
    ["desert", "Desert", "#a95a14"], ["forest", "Forest", "#4caf6a"], ["sunny", "Sunny", "#f5a000"],
    ["orange", "Orange", "#c94a05"], ["leaf", "Leaf", null], ["earth", "Earth", null],
    ["sky", "Sky", null], ["river", "River", null], ["water", "Water", null],
  ];
  const root = document.documentElement;
  const KEY = "mohona.site.theme";
  const store = {
    get() { try { return localStorage.getItem(KEY); } catch { return null; } },
    set(v) { try { localStorage.setItem(KEY, v); } catch { /* private mode */ } },
  };
  const dark = () => matchMedia("(prefers-color-scheme: dark)").matches;
  const shotFor = (t) => `shots/main-${t === "system" ? (dark() ? "dark" : "light") : t}.webp`;

  // Theme picker -----------------------------------------------------------
  const select = document.getElementById("theme");
  const swatches = document.getElementById("swatches");
  let current = store.get();
  if (!THEMES.some(([id]) => id === current)) current = "river";
  function apply(id) {
    current = id;
    if (id === "system") root.removeAttribute("data-theme"); else root.setAttribute("data-theme", id);
    store.set(id);
    select.value = id;
    document.querySelectorAll(".sw").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.id === id)));
    for (const el of [document.getElementById("hero-shot"), document.getElementById("theme-shot")]) el.src = shotFor(id);
  }
  for (const [id, label] of THEMES) select.add(new Option(label, id));
  select.addEventListener("change", () => apply(select.value));
  for (const [id, label] of THEMES) {
    const b = document.createElement("button");
    b.className = "sw"; b.dataset.id = id; b.type = "button";
    b.innerHTML = `<i></i>${label}`;
    b.addEventListener("click", () => apply(id));
    swatches.append(b);
  }
  apply(current);
  // Fill each swatch with the theme's own accent, read from the page's CSS.
  for (const b of swatches.children) {
    const id = b.dataset.id;
    const probe = document.createElement("div");
    probe.style.display = "none"; document.body.append(probe);
    const was = root.getAttribute("data-theme");
    if (id === "system") root.removeAttribute("data-theme"); else root.setAttribute("data-theme", id);
    b.firstChild.style.background = getComputedStyle(root).getPropertyValue("--accent").trim();
    if (was) root.setAttribute("data-theme", was); else root.removeAttribute("data-theme");
    probe.remove();
  }

  // Screenshot gallery -------------------------------------------------------
  const shots = [
    ["tab1", () => shotFor(current), "cap1"],
    ["tab2", "shots/add-river.webp", "cap2"],
    ["tab3", "shots/grabber-river.webp", "cap3"],
    ["tab4", "shots/settings-queues-river.webp", "cap4"],
    ["tab5", "shots/settings-schedule-river.webp", "cap5"],
    ["tab6", "shots/settings-torrents-river.webp", "cap6"],
    ["tab7", "shots/extension-popup-river.webp", "cap7", true],
    ["tab8", "shots/cli-get.webp", "cap8", false, true],
    ["tab9", "shots/settings-dark.webp", "cap9"],
  ];
  let shown = 0;
  const tabs = document.getElementById("tabs");
  const img = document.getElementById("stage-img");
  const frame = document.getElementById("stage-frame");
  const cap = document.getElementById("stage-cap");
  function show(i) {
    shown = i;
    const [, src, capKey, narrow, bare] = shots[i];
    const text = t(capKey);
    const s = typeof src === "function" ? src() : src;
    img.src = s; img.alt = text; cap.textContent = text;
    frame.classList.toggle("narrow", !!narrow);
    frame.querySelector(".chrome").style.display = narrow || bare ? "none" : "";
    [...tabs.children].forEach((t, k) => t.setAttribute("aria-selected", String(k === i)));
  }
  shots.forEach(([label], i) => {
    const b = document.createElement("button");
    b.className = "tab"; b.type = "button"; b.role = "tab"; b.dataset.k = label;
    b.addEventListener("click", () => show(i));
    tabs.append(b);
  });
  rerender.push(() => { [...tabs.children].forEach((b) => (b.textContent = t(b.dataset.k))); show(shown); });
  show(0);
  const lb = document.getElementById("lb");
  img.style.cursor = "zoom-in";
  img.addEventListener("click", () => { lb.querySelector("img").src = img.src; lb.showModal(); });
  lb.addEventListener("click", () => lb.close());
  select.addEventListener("change", () => { if (tabs.firstChild.getAttribute("aria-selected") === "true") show(0); });
  swatches.addEventListener("click", () => { if (tabs.firstChild.getAttribute("aria-selected") === "true") show(0); });

  // Reveal on scroll ---------------------------------------------------------
  const io = "IntersectionObserver" in window ? new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); } }), { threshold: 0.12 }) : null;
  document.querySelectorAll(".reveal").forEach((el) => (io ? io.observe(el) : el.classList.add("in")));

  // Releases -----------------------------------------------------------------
  const os = /Windows/i.test(navigator.userAgent) ? "windows" : /Mac/i.test(navigator.userAgent) ? "mac" : /Android/i.test(navigator.userAgent) ? "android" : /Linux|X11/i.test(navigator.userAgent) ? "linux" : "other";
  const kind = (n) => {
    const l = n.toLowerCase();
    if (/\.(sha256|sha512|sig|asc)$/.test(l) || l === "sha256sums.txt") return null;
    if (l.endsWith(".exe") || l.endsWith(".msi")) return ["Windows", "windows", l.endsWith(".msi") ? "a_msi" : "a_installer"];
    if (l.endsWith(".dmg")) return ["macOS", "mac", "a_dmg"];
    if (l.endsWith(".appimage")) return ["Linux", "linux", "a_appimage"];
    if (l.endsWith(".deb")) return ["Linux", "linux", "a_deb"];
    if (l.endsWith(".rpm")) return ["Linux", "linux", "a_rpm"];
    if (/chromium.*\.zip$/.test(l)) return [null, "ext", "a_chromium"];
    if (/firefox.*\.(zip|xpi)$/.test(l)) return [null, "ext", "a_firefox"];
    return null;
  };
  let release = null;
  function paintRelease() {
    const rel = release;
    if (!rel) return;
    const tag = rel.tag_name || "";
    document.getElementById("ver").textContent = tag;
    const list = [];
    for (const a of rel.assets || []) { const k = kind(a.name); if (k) list.push({ a, k }); }
    if (!list.length) return;
    const name = (k) => k[0] || t("a_ext");
    list.sort((x, y) => (x.k[1] === os ? -1 : 0) - (y.k[1] === os ? -1 : 0) || name(x.k).localeCompare(name(y.k)));
    const box = document.getElementById("assets");
    box.replaceChildren();
    list.forEach(({ a, k }, i) => {
      const el = document.createElement("a");
      el.className = "asset" + (i === 0 && k[1] === os ? " main" : "");
      el.href = a.browser_download_url;
      el.innerHTML = `<b></b><span></span>`;
      el.firstChild.textContent = `${name(k)} · ${t(k[2])}`;
      el.lastChild.textContent = `${a.name} · ${size(a.size)}`;
      box.append(el);
    });
    const more = document.createElement("a");
    more.className = "asset"; more.href = rel.html_url;
    more.innerHTML = "<b></b><span></span>";
    more.firstChild.textContent = t("dl_allfiles"); more.lastChild.textContent = t("dl_ongh");
    box.append(more);
    const mine = list.find(({ k }) => k[1] === os);
    const hero = document.getElementById("hero-dl");
    if (mine) { hero.href = mine.a.browser_download_url; hero.textContent = `⬇ ${t("dl_for", { os: mine.k[0] })} · ${tag}`; }
    else { hero.href = rel.html_url; hero.textContent = `⬇ ${t("dl_tag", { tag })}`; }
  }
  rerender.push(paintRelease);
  const size = (b) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : Math.round(b / 1e3) + " KB");
  fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
    .then((rel) => { release = rel; paintRelease(); })
    .catch(() => { /* no release yet, or offline: the plain links stay */ });

  // Streams that flow down and meet at the river mouth, like the icon's arrow into the water --
  const cv = document.getElementById("streams");
  const still = matchMedia("(prefers-reduced-motion: reduce)");
  if (cv && cv.getContext) {
    const g = cv.getContext("2d");
    let w = 0, h = 0, dpr = 1, parts = [], run = true;
    const curves = [];
    function size2() {
      const r = cv.getBoundingClientRect();
      dpr = Math.min(devicePixelRatio || 1, 2); w = r.width; h = r.height;
      cv.width = w * dpr; cv.height = h * dpr; g.setTransform(dpr, 0, 0, dpr, 0, 0);
      curves.length = 0;
      const mouthX = w * 0.5, mouthY = h * 0.86, n = w < 700 ? 5 : 9;
      for (let i = 0; i < n; i++) {
        const sx = (w * (i + 0.5)) / n, sy = -10 + (i % 3) * 6;
        const bend = (i - (n - 1) / 2) * (w * 0.035);
        curves.push([sx, sy, sx + bend, h * 0.38, mouthX + (sx - mouthX) * 0.25, h * 0.62, mouthX, mouthY]);
      }
      parts = [];
      for (let i = 0; i < n * 14; i++) parts.push({ c: i % n, p: Math.random(), v: 0.0012 + Math.random() * 0.0022, s: 1 + Math.random() * 2.2 });
    }
    const at = (c, p) => {
      const [x0, y0, x1, y1, x2, y2, x3, y3] = c, q = 1 - p;
      return [q * q * q * x0 + 3 * q * q * p * x1 + 3 * q * p * p * x2 + p * p * p * x3, q * q * q * y0 + 3 * q * q * p * y1 + 3 * q * p * p * y2 + p * p * p * y3];
    };
    function frame() {
      g.clearRect(0, 0, w, h);
      g.lineWidth = 1.2; g.strokeStyle = "rgba(160,235,235,.16)";
      for (const [x0, y0, x1, y1, x2, y2, x3, y3] of curves) { g.beginPath(); g.moveTo(x0, y0); g.bezierCurveTo(x1, y1, x2, y2, x3, y3); g.stroke(); }
      for (const q of parts) {
        if (!still.matches) { q.p += q.v; if (q.p > 1) { q.p = 0; } }
        const [x, y] = at(curves[q.c], q.p);
        const a = Math.sin(Math.PI * q.p) * 0.9;
        g.fillStyle = `rgba(190,255,245,${a})`;
        g.beginPath(); g.arc(x, y, q.s, 0, 6.2832); g.fill();
      }
      // ripples where the streams reach the water
      const t0 = performance.now() / 1000;
      for (let k = 0; k < 3; k++) {
        const ph = (t0 / 3 + k / 3) % 1;
        g.strokeStyle = `rgba(160,245,235,${0.35 * (1 - ph)})`; g.lineWidth = 1.6;
        g.beginPath(); g.ellipse(w * 0.5, h * 0.86, 20 + ph * 150, 6 + ph * 34, 0, 0, 6.2832); g.stroke();
      }
      if (run && !still.matches) requestAnimationFrame(frame);
    }
    size2(); frame();
    addEventListener("resize", () => { size2(); if (still.matches) frame(); });
    document.addEventListener("visibilitychange", () => { run = !document.hidden; if (run && !still.matches) requestAnimationFrame(frame); });
  }

  // Boot: pick the language, then paint everything -------------------------------
  Promise.all([load("en").catch(() => ({}))]).then(([en]) => { base = en; return setLang(detect(), false); });
})();
