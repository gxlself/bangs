// The page shows the app rather than describing it: the notch opens and closes
// on its own, the lyric fills in word by word, and the download buttons point
// at whatever the newest GitHub release actually contains.

const REPO = "gxlself/bangs";
const RELEASES = `https://github.com/${REPO}/releases`;

/* ---------- The notch demo ---------- */

const notch = document.querySelector("[data-notch]");
const compactLyric = document.querySelector("[data-lyric]");
const panelLyric = document.querySelector("[data-panel-lyric]");
const progress = document.querySelector("[data-progress]");

const LINE = "我曾将青春翻涌成她";
const COMPACT_LINE = "也曾指尖弹出盛夏";

/** Fills a line in from the left, one character at a time. */
function karaoke(element, text, sung) {
  const cut = Math.max(0, Math.min(text.length, Math.round(sung * text.length)));
  element.innerHTML = `<b>${text.slice(0, cut)}</b><span>${text.slice(cut)}</span>`;
}

let started = performance.now();
function frame(now) {
  const beat = ((now - started) / 4200) % 1;
  if (compactLyric) karaoke(compactLyric, COMPACT_LINE, beat);
  if (panelLyric) karaoke(panelLyric, LINE, beat);
  if (progress) progress.style.width = `${18 + beat * 52}%`;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// Open, hold, close — and let the pointer take over when it arrives.
let auto = true;
let phase = 0;
setInterval(() => {
  if (!auto || !notch) return;
  phase = (phase + 1) % 2;
  notch.classList.toggle("is-open", phase === 1);
}, 3600);

if (notch) {
  const screen = notch.closest(".screen__body");
  screen?.addEventListener("pointerenter", () => {
    auto = false;
    notch.classList.add("is-open");
  });
  screen?.addEventListener("pointerleave", () => {
    auto = true;
    phase = 0;
    notch.classList.remove("is-open");
  });
}

/* ---------- Feature card karaoke ---------- */

const cardLine = document.querySelector("[data-karaoke] span");
if (cardLine) {
  const text = cardLine.textContent.trim();
  const holder = cardLine.parentElement;
  const tick = (now) => {
    karaoke(holder, text, ((now / 3400) % 1));
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/* ---------- Reveal on scroll ---------- */

const observer = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      }
    }
  },
  { rootMargin: "-40px" },
);
document.querySelectorAll(".card, .release, .code, .section__title").forEach((node) => observer.observe(node));

/* ---------- Chinese or English ---------- */

// Everything translatable carries its English in data-en; the Chinese it ships
// with is kept the first time it is swapped out.
const translatable = document.querySelectorAll("[data-en]");
const toggle = document.querySelector("[data-lang-toggle]");
let english = !/^zh/i.test(navigator.language || "");

function paint() {
  for (const node of translatable) {
    if (node.dataset.zh === undefined) node.dataset.zh = node.innerHTML;
    node.innerHTML = english ? node.dataset.en : node.dataset.zh;
  }
  document.documentElement.lang = english ? "en" : "zh-CN";
  if (toggle) toggle.textContent = english ? "中文" : "EN";
}

toggle?.addEventListener("click", () => {
  english = !english;
  paint();
  loadRelease();
});
paint();

/* ---------- The latest release ---------- */

// In preference order: the friendly installer first, the alternative second.
const MAC = [/\.dmg$/i, /\.app\.zip$/i];
const WINDOWS = [/setup\.exe$/i, /\.msi$/i];

function bytes(size) {
  return `${(size / 1024 / 1024).toFixed(1)} MB`;
}

const say = (zh, en) => (english ? en : zh);

function applyAsset(patterns, links, meta, suffix) {
  return (release) => {
    const asset = patterns
      .map((pattern) => release.assets.find((item) => pattern.test(item.name)))
      .find(Boolean);
    for (const link of links) {
      link.href = asset ? asset.browser_download_url : `${RELEASES}/latest`;
    }
    if (meta) {
      meta.textContent = asset ? `${suffix} · ${bytes(asset.size)}` : `${suffix} · ${say("见 Releases", "see Releases")}`;
    }
  };
}

async function loadRelease() {
  const text = document.querySelector("[data-version-text]");
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(String(response.status));
    const release = await response.json();
    const version = (release.tag_name || "").replace(/^v/, "");

    const published = new Date(release.published_at).toLocaleDateString(english ? "en-GB" : "zh-CN");
    if (text) text.textContent = say(`最新版本 v${version} · ${published}`, `Latest release v${version} · ${published}`);

    applyAsset(
      MAC,
      [document.querySelector("[data-download-mac]"), document.querySelector("[data-mac-link]")].filter(Boolean),
      document.querySelector("[data-mac-meta]"),
      say("Apple 芯片 · macOS 12+", "Apple silicon · macOS 12+"),
    )(release);

    applyAsset(
      WINDOWS,
      [document.querySelector("[data-download-win]"), document.querySelector("[data-win-link]")].filter(Boolean),
      document.querySelector("[data-win-meta]"),
      say("Windows 10 / 11 · 需要 WebView2", "Windows 10 / 11 · needs WebView2"),
    )(release);

    const note = document.querySelector("[data-download-note]");
    if (note) note.textContent = say(`${release.assets.length} 个安装包 · 两个平台同一套代码`, `${release.assets.length} downloads · one codebase, both platforms`);
  } catch {
    // No releases yet, or GitHub is rate limiting: the plain links still work.
    if (text) text.textContent = say("从 GitHub Releases 下载", "Download from GitHub Releases");
    document
      .querySelectorAll("[data-download-mac], [data-download-win], [data-mac-link], [data-win-link]")
      .forEach((link) => {
        link.href = `${RELEASES}/latest`;
      });
  }
}

loadRelease();
