// A file tab's page: draws diagrams and math with the downloaded viewers, keeps up with the
// file, and hands links to macaroni. Settings arrive in window.MACARONI (see
// crates/app/src/preview/mod.rs); each viewer's script loads only when the page needs it.

const settings = window.MACARONI;
const viewers = settings.viewers;
const doc = document.getElementById("mx-doc");

// Each viewer's loaded library, by name, as a promise.
const loaded = {};

// Mermaid needs a unique id for every diagram it draws.
let drawn = 0;

/** Loads a classic script. */
function script(src) {
  return new Promise((resolve, reject) => {
    const element = document.createElement("script");
    element.src = src;
    element.onload = resolve;
    element.onerror = () => reject(new Error(`Couldn't load ${src}`));
    document.head.append(element);
  });
}

/** Adds a stylesheet. */
function stylesheet(href) {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = href;
  document.head.append(link);
}

/** The library `name`, loading it the first time. */
function library(name, load) {
  loaded[name] ??= load();
  return loaded[name];
}

// Each viewer draws a diagram's source into an SVG (an element or its markup).
const draw = {
  mermaid: async (source) => {
    const mermaid = await library("mermaid", async () => {
      await script(`${viewers.mermaid}dist/mermaid.min.js`);
      window.mermaid.initialize({
        startOnLoad: false,
        theme: settings.dark ? "dark" : "default",
        securityLevel: "strict",
      });
      return window.mermaid;
    });
    const { svg } = await mermaid.render(`mx-mermaid-${++drawn}`, source);
    return svg;
  },
  graphviz: async (source) => {
    const viz = await library("graphviz", async () => {
      await script(`${viewers.graphviz}dist/viz-global.js`);
      return window.Viz.instance();
    });
    // Graphviz draws on white in black unless told; the source's own colors still win.
    const ink = { color: settings.ink, fontcolor: settings.ink };
    return viz.renderSVGElement(source, {
      graphAttributes: { bgcolor: "transparent", ...ink },
      nodeAttributes: ink,
      edgeAttributes: ink,
    });
  },
  plantuml: async (source) => {
    const plantuml = await library("plantuml", async () => {
      await script(`${viewers.plantuml}viz-global.js`);
      // Importing the themes registers them for `!theme`.
      await import(`${viewers.plantuml}themes.js`);
      return import(`${viewers.plantuml}plantuml.js`);
    });
    const text = /@start\w+/.test(source) ? source : `@startuml\n${source}\n@enduml`;
    return new Promise((resolve, reject) => {
      plantuml.renderToString(
        text.split(/\r?\n/),
        resolve,
        (error) => reject(new Error(error)),
        { dark: settings.dark },
      );
    });
  },
  d2: async (source) => {
    const d2 = await library("d2", async () => {
      const { D2 } = await import(`${viewers.d2}dist/browser/index.js`);
      return new D2();
    });
    const result = await d2.compile(source);
    // 200 is D2's "Dark Mauve", its default dark theme. Without a scale D2 stretches the
    // drawing to fill the page, so it's drawn at its own size unless the source says.
    const options = result.renderOptions ?? {};
    return d2.render(result.diagram, {
      ...options,
      scale: options.scale > 0 ? options.scale : 1,
      pad: options.pad ?? 24,
      themeID: settings.dark ? 200 : (options.themeID ?? 0),
    });
  },
};

/** Typesets every math span under `root` with KaTeX, when it's installed. */
async function typeset(root) {
  const spans = [...root.querySelectorAll(".mx-math")];
  if (!spans.length || !viewers.math) {
    return;
  }
  const katex = await library("math", async () => {
    stylesheet(`${viewers.math}dist/katex.min.css`);
    await script(`${viewers.math}dist/katex.min.js`);
    return window.katex;
  });
  for (const span of spans) {
    katex.render(span.textContent, span, {
      displayMode: span.classList.contains("mx-display"),
      throwOnError: false,
    });
    span.classList.add("mx-typeset");
  }
}

/** Gives headings without an id GitHub's anchor (lowercase, punctuation dropped, dashes). */
function anchor(root) {
  const seen = new Map();
  for (const heading of root.querySelectorAll("h1, h2, h3, h4, h5, h6")) {
    if (heading.id) {
      continue;
    }
    const slug = heading.textContent
      .trim()
      .toLowerCase()
      .replace(/[^\p{L}\p{N}\s_-]/gu, "")
      .replace(/\s/g, "-");
    const count = seen.get(slug) ?? 0;
    seen.set(slug, count + 1);
    heading.id = count ? `${slug}-${count}` : slug;
  }
}

/** Draws one diagram's figure with its viewer, or adds the viewer's error under its source. */
async function drawFigure(figure) {
  const viewer = draw[figure.dataset.kind];
  const source = figure.querySelector(".mx-source")?.textContent ?? "";
  if (!viewer) {
    return;
  }
  try {
    const svg = await viewer(source);
    const view = document.createElement("div");
    view.className = "mx-drawing";
    if (typeof svg === "string") {
      view.innerHTML = svg;
    } else {
      view.append(svg);
    }
    figure.replaceChildren(view);
    figure.classList.add("mx-drawn");
  } catch (error) {
    const message = document.createElement("pre");
    message.className = "mx-error";
    message.textContent = String(error?.message ?? error);
    figure.append(message);
  }
}

/** Draws everything under `root`: anchors, then math and every diagram at once, so a slow
 * viewer doesn't hold up the others. */
async function render(root) {
  anchor(root);
  const math = typeset(root).catch((error) => console.error(error));
  const figures = [...root.querySelectorAll(".mx-diagram[data-kind]")].map(drawFigure);
  await Promise.all([math, ...figures]);
}

/** Media the web view can't decode says so in place of a broken player. */
function watchMedia(root) {
  for (const element of root.querySelectorAll(".mx-media img, .mx-media video, .mx-media audio")) {
    element.addEventListener("error", () => {
      const note = document.createElement("p");
      note.className = "mx-note";
      note.textContent =
        "This web view can't play or show this file's format. Open it in another app instead.";
      element.replaceWith(note);
    });
  }
}

/** Fetches the new body, draws it out of sight, then swaps it in where the reader was (or at
 * the top, for another file). */
async function refresh(top) {
  const response = await fetch(`${settings.page}/body`, { cache: "no-store" });
  if (!response.ok) {
    return;
  }
  const staging = document.createElement("main");
  staging.className = "mx-staging";
  staging.innerHTML = await response.text();
  document.body.append(staging);
  await render(staging);
  watchMedia(staging);
  const y = top ? 0 : window.scrollY;
  doc.replaceChildren(...staging.childNodes);
  staging.remove();
  window.scrollTo(0, y);
}

/** Checks for a new version every so often: a new body is swapped in, a new frame reloads. */
async function poll() {
  try {
    const response = await fetch(`${settings.page}/version`, { cache: "no-store" });
    if (response.ok) {
      const [version, sig, file] = (await response.text()).split(" ");
      if (sig !== settings.sig) {
        // The reload keeps the place only when it's the same file.
        if (file === settings.doc) {
          remember();
        }
        location.reload();
        return;
      }
      if (version !== settings.version) {
        const other = file !== settings.doc;
        settings.version = version;
        settings.doc = file;
        await refresh(other);
      }
    }
  } catch (error) {
    console.error(error);
  }
  setTimeout(poll, 700);
}

// Links: `#section` scrolls here (the page's <base> is the document's folder, so the browser
// would leave the page), anything else goes to macaroni, which opens files in tabs and web
// addresses in the browser.
document.addEventListener("click", (event) => {
  const link = event.target.closest("a");
  const href = link?.getAttribute("href") ?? link?.getAttribute("xlink:href");
  if (!href || event.defaultPrevented) {
    return;
  }
  event.preventDefault();
  if (href.startsWith("#")) {
    const id = decodeURIComponent(href.slice(1));
    (document.getElementById(id) ?? document.getElementsByName(id)[0])?.scrollIntoView();
    return;
  }
  fetch(`${settings.page}/link`, { method: "POST", body: href }).catch(() => {});
});

// Where the reader was, kept across a reload (a new look) of the same file.
const PLACE = "macaroni-place";

/** Remembers the scroll position for the reload that follows. */
function remember() {
  try {
    sessionStorage.setItem(PLACE, JSON.stringify({ doc: settings.doc, y: window.scrollY }));
  } catch {}
}

/** The remembered position, if it was this file's, forgotten once read. */
function recall() {
  try {
    const place = JSON.parse(sessionStorage.getItem(PLACE) ?? "null");
    sessionStorage.removeItem(PLACE);
    return place?.doc === settings.doc ? place.y : 0;
  } catch {
    return 0;
  }
}

history.scrollRestoration = "manual";
watchMedia(doc);
render(doc)
  .then(() => window.scrollTo(0, recall()))
  .finally(() => setTimeout(poll, 700));
