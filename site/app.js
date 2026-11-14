(() => {
  "use strict";
  const REPO = "arifulalam/mohona";
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
    ["Downloads", () => shotFor(current), "Your list: progress, speed and time left, with categories on the left."],
    ["Add", "shots/add-river.webp", "Paste web addresses, magnet links, video pages or a Google Drive folder."],
    ["Site Grabber", "shots/grabber-river.webp", "Find every file a site offers and pick what to download."],
    ["Queues", "shots/settings-queues-river.webp", "Run lists with limits on how many go at once."],
    ["Schedule", "shots/settings-schedule-river.webp", "Download at night or on weekdays only, and shut down when done."],
    ["Torrents", "shots/settings-torrents-river.webp", "Control sharing and set separate torrent speed limits."],
    ["Browser extension", "shots/extension-popup-river.webp", "The popup: add links, see progress, and choose quality and format for a video.", true],
    ["Command line", "shots/cli-get.webp", "The mohona command: get, queue, grab, video-format, scan and more.", false, true],
    ["Dark theme", "shots/settings-dark.webp", "Light, dark and nine other themes, all consistent."],
  ];
  const tabs = document.getElementById("tabs");
  const img = document.getElementById("stage-img");
  const frame = document.getElementById("stage-frame");
  const cap = document.getElementById("stage-cap");
  function show(i) {
    const [, src, text, narrow, bare] = shots[i];
    const s = typeof src === "function" ? src() : src;
    img.src = s; img.alt = text; cap.textContent = text;
    frame.classList.toggle("narrow", !!narrow);
    frame.querySelector(".chrome").style.display = narrow || bare ? "none" : "";
    [...tabs.children].forEach((t, k) => t.setAttribute("aria-selected", String(k === i)));
  }
  shots.forEach(([label], i) => {
    const t = document.createElement("button");
    t.className = "tab"; t.type = "button"; t.role = "tab"; t.textContent = label;
    t.addEventListener("click", () => show(i));
    tabs.append(t);
  });
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
    if (l.endsWith(".exe") || l.endsWith(".msi")) return ["Windows", "windows", l.endsWith(".msi") ? "Installer (MSI)" : "Installer"];
    if (l.endsWith(".dmg")) return ["macOS", "mac", "Disk image"];
    if (l.endsWith(".appimage")) return ["Linux", "linux", "AppImage — runs anywhere"];
    if (l.endsWith(".deb")) return ["Linux", "linux", "Debian / Ubuntu package"];
    if (l.endsWith(".rpm")) return ["Linux", "linux", "Fedora / openSUSE package"];
    if (/chromium.*\.zip$/.test(l)) return ["Extension", "ext", "Chrome / Edge / Brave"];
    if (/firefox.*\.(zip|xpi)$/.test(l)) return ["Extension", "ext", "Firefox"];
    return null;
  };
  const size = (b) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : Math.round(b / 1e3) + " KB");
  fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
    .then((rel) => {
      const tag = rel.tag_name || "";
      document.getElementById("ver").textContent = tag;
      const list = [];
      for (const a of rel.assets || []) { const k = kind(a.name); if (k) list.push({ a, k }); }
      if (!list.length) return;
      list.sort((x, y) => (x.k[1] === os ? -1 : 0) - (y.k[1] === os ? -1 : 0) || x.k[0].localeCompare(y.k[0]));
      const box = document.getElementById("assets");
      box.replaceChildren();
      list.forEach(({ a, k }, i) => {
        const el = document.createElement("a");
        el.className = "asset" + (i === 0 && k[1] === os ? " main" : "");
        el.href = a.browser_download_url;
        el.innerHTML = `<b></b><span></span>`;
        el.firstChild.textContent = `${k[0]} · ${k[2]}`;
        el.lastChild.textContent = `${a.name} · ${size(a.size)}`;
        box.append(el);
      });
      const more = document.createElement("a");
      more.className = "asset"; more.href = rel.html_url;
      more.innerHTML = "<b>All files, notes and checksums</b><span>on GitHub Releases</span>";
      box.append(more);
      const mine = list.find(({ k }) => k[1] === os);
      const hero = document.getElementById("hero-dl");
      if (mine) { hero.href = mine.a.browser_download_url; hero.textContent = `⬇ Download for ${mine.k[0]} · ${tag}`; }
      else { hero.href = rel.html_url; hero.textContent = `⬇ Download ${tag}`; }
    })
    .catch(() => { /* no release yet, or offline: the plain links stay */ });
})();
