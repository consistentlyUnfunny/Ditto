/**
 * Ditto — desktop UI.
 *
 * Every decision lives in the Rust engine; this file asks questions and draws
 * the answers. In particular the deploy preview is rendered from the same plan
 * the apply call executes, so what you see is what gets written.
 */

// ---------------------------------------------------------------------------
// Types — mirrors of crates/ditto-gui/src/dto.rs
// ---------------------------------------------------------------------------

type Provider = { id: string; name: string; installed: boolean; home: string | null };
type Cell = { state: "on" | "off" | "bad"; path: string | null };
type Tie = { from: number; to: number; drift: boolean; shared: boolean };
type Row = {
  name: string;
  kind: "skill" | "server";
  status: "unique" | "identical" | "drifted" | "problem";
  detail: string | null;
  cells: Cell[];
  ties: Tie[];
  deprecated: boolean;
  deprecated_path: string | null;
  /** The library's own cell, kept out of `cells` because it is not a harness. */
  library: Cell;
};
type Project = {
  path: string;
  name: string;
  is_git: boolean;
  sources: string[];
  markers: string[];
  /** You added this one, which is also the only case where it can be removed. */
  added: boolean;
};
type LibraryInfo = {
  enabled: boolean;
  path: string | null;
  /** Whether that directory is there yet; nothing creates it until it is used. */
  exists: boolean;
  settings_path: string | null;
};
type Overview = {
  scope: string;
  /** Draw the library column: on at user scope when the library is enabled. */
  library: boolean;
  project: Project | null;
  providers: Provider[];
  skills: Row[];
  servers: Row[];
};

type Copy = {
  path: string; providers: string[]; description: string; body: string;
  hash: string; deprecated: boolean; readable: boolean;
  changed: string | null; library: boolean;
};
type Side = {
  path: string; providers: string[]; hash: string;
  changed: string | null; library: boolean;
};
type Comparison = { left: Side; right: Side; lines: DiffLine[] };
type Version = {
  hash: string;
  description: string;
  providers: string[];
  paths: string[];
  source: string;
  is_default: boolean;
  /** "3 days ago" — shown while choosing, never used to choose. */
  changed: string | null;
  library: boolean;
};
type Target = { id: string; name: string; has_it: boolean };
type SkillDetail = {
  kind: "skill";
  name: string;
  status: string;
  copies: Copy[];
  comparison: Comparison | null;
  source: string | null;
  version_count: number;
  unusable: boolean;
  versions: Version[];
  targets: Target[];
  in_library: boolean;
  library_enabled: boolean;
};
type ServerDetail = {
  kind: "server";
  name: string;
  provider: string;
  transport: string;
  command: string;
  url: string | null;
  env_names: string[];
  secret_names: string[];
  extra_keys: string[];
  file: string;
  scope: string;
  project: string | null;
};
type Detail = SkillDetail | ServerDetail;

type DiffLine = { kind: "add" | "del" | "ctx"; text: string };
type Step = {
  kind: "config" | "skill" | "config-remove" | "skill-remove";
  target: string; providers: string[]; reason: string;
  diff: DiffLine[]; dropped: string[]; warnings: string[];
  /** The git working tree this write lands in, if any. */
  git: string | null;
};
type PlanView = {
  steps: Step[];
  refusals: { provider: string; reason: string }[];
  served: string[];
};
type Outcome = { target: string; providers: string[]; ok: boolean; message: string };

// ---------------------------------------------------------------------------
// Bridge
// ---------------------------------------------------------------------------

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const bridge = window.__TAURI__?.core?.invoke;
  if (!bridge) {
    throw new Error("Not running inside the app window.");
  }
  return (await bridge(cmd, args)) as T;
}

// ---------------------------------------------------------------------------
// Small DOM helpers — enough structure to avoid string-concatenating HTML.
// ---------------------------------------------------------------------------

/**
 * Line icons, drawn on a 16x16 grid and stroked in `currentColor` so they take
 * the colour of whatever they sit in — including across a theme change, which
 * a bitmap or a fill-coloured SVG would not.
 *
 * Inline rather than a sprite sheet or an icon package: this window ships no
 * runtime dependencies and makes no requests, and six paths do not justify
 * breaking either rule.
 */
const ICONS = {
  refresh: ["M14 8a6 6 0 1 1-1.76-4.24", "M14 2.5V6h-3.5"],
  sun: [
    "M8 5.25a2.75 2.75 0 1 0 0 5.5 2.75 2.75 0 0 0 0-5.5Z",
    "M8 1v1.5M8 13.5V15M15 8h-1.5M2.5 8H1M12.95 3.05l-1.06 1.06M4.11 11.89l-1.06 1.06M12.95 12.95l-1.06-1.06M4.11 4.11 3.05 3.05",
  ],
  moon: ["M13.5 9.6A5.6 5.6 0 0 1 6.4 2.5a5.75 5.75 0 1 0 7.1 7.1Z"],
  person: ["M13 14v-1.5A3.5 3.5 0 0 0 9.5 9h-3A3.5 3.5 0 0 0 3 12.5V14", "M8 6.5a2.75 2.75 0 1 0 0-5.5 2.75 2.75 0 0 0 0 5.5Z"],
  folder: ["M2 4.5A1.5 1.5 0 0 1 3.5 3h2.2a1 1 0 0 1 .8.4l.9 1.2a1 1 0 0 0 .8.4h4.3A1.5 1.5 0 0 1 14 6.5v5A1.5 1.5 0 0 1 12.5 13h-9A1.5 1.5 0 0 1 2 11.5v-7Z"],
  caret: ["m4 6.5 4 4 4-4"],
} as const;

function icon(name: keyof typeof ICONS, className?: string): SVGSVGElement {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.5");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  // Decoration: the button around it carries the name for a screen reader.
  svg.setAttribute("aria-hidden", "true");
  if (className) svg.setAttribute("class", className);
  for (const d of ICONS[name]) {
    const path = document.createElementNS(NS, "path");
    path.setAttribute("d", d);
    svg.appendChild(path);
  }
  return svg;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function clear(node: HTMLElement) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

const main = document.getElementById("main") as HTMLElement;
const drawer = document.getElementById("drawer") as HTMLElement;
const scrim = document.getElementById("scrim") as HTMLElement;
const scopeLabel = document.getElementById("scope") as HTMLButtonElement;
const tabs = document.getElementById("tabs") as HTMLElement;
const refreshButton = document.getElementById("refresh") as HTMLButtonElement;
const themeButton = document.getElementById("theme") as HTMLButtonElement;

let current: Overview | null = null;

/** Who holds a copy. The library is read by no harness, so its list is empty. */
function holder(it: { providers: string[]; library: boolean }): string {
  return it.library ? "your library" : it.providers.join(", ");
}

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------

/**
 * Column centres sit at (i + 0.5) / n across the cells track, so a tie between
 * columns a and b starts at a's centre and runs (b - a) columns wide.
 */
function tieStyle(tie: Tie, columns: number): string {
  const left = ((tie.from + 0.5) / columns) * 100;
  const width = ((tie.to - tie.from) / columns) * 100;
  return `left:${left}%; width:${width}%;`;
}

function renderRow(
  row: Row,
  providers: Provider[],
  onOpen: (name: string) => void,
  showLibrary: boolean,
) {
  const frag = document.createDocumentFragment();

  const button = el("button", "arow");
  button.type = "button";
  button.addEventListener("click", () => onOpen(row.name));

  const label = el("span", row.status === "problem" ? "rowname dim" : "rowname");
  label.appendChild(el("span", "rowtext", row.name));
  // The CLI has always printed "!! deprecated path"; the window hid it, which
  // made the most actionable finding on a machine invisible here.
  if (row.deprecated) {
    // A compact mark, not a pill. On this machine six of seven skills sit in a
    // deprecated location, and six repeated badges crowded out the names — the
    // thing the column is actually for. `doctor` states it once, in words.
    const flag = el("span", "rowmark");
    flag.title = `in a deprecated location
${row.deprecated_path ?? ""}`;
    label.appendChild(flag);
  }
  // Something to say the row is openable, rather than relying on the cursor.
  label.appendChild(el("span", "chev", "›"));
  label.title = row.name;
  button.appendChild(label);

  // Its own column, outside the harness track. The tie geometry below divides
  // the track by the number of harnesses, so folding the library in would shift
  // every staple by half a column.
  if (showLibrary) {
    const lib = el("span", "libcell");
    const dot = el("i", `dot ${row.library.state}`);
    dot.title = row.library.path
      ? `your library
${row.library.path}`
      : row.kind === "server"
        ? "servers cannot be imported yet"
        : "not in your library";
    lib.appendChild(dot);
    button.appendChild(lib);
  }

  const cells = el("span", "cells");
  cells.style.gridTemplateColumns = `repeat(${providers.length}, 1fr)`;

  for (const tie of row.ties) {
    const bar = el("span", `tie${tie.shared ? " shared" : ""}${tie.drift ? " drift" : ""}`);
    bar.setAttribute("style", tieStyle(tie, providers.length));
    cells.appendChild(bar);
  }

  // Break the staple wherever it passes a harness that has nothing to do with
  // it. Codex and OpenCode sit four columns apart, and an unbroken line across
  // that gap reads as "all of these differ" — which is the opposite of true.
  for (const tie of row.ties) {
    for (let i = tie.from + 1; i < tie.to; i += 1) {
      const gap = el("span", "tie-gap");
      gap.setAttribute("style", `left:${((i + 0.5) / providers.length) * 100}%;`);
      cells.appendChild(gap);
    }
  }

  row.cells.forEach((cell, index) => {
    const wrap = el("span", "cell");
    const dot = el("i", `dot ${cell.state}`);
    const provider = providers[index]?.name ?? "";
    dot.title = cell.path ? `${provider}\n${cell.path}` : `not installed for ${provider}`;
    wrap.appendChild(dot);
    cells.appendChild(wrap);
  });

  button.appendChild(cells);
  frag.appendChild(button);

  if (row.detail) {
    const note = el("div", row.status === "drifted" || row.status === "problem"
      ? "mrow detailrow"
      : "mrow detailrow quiet");
    const inner = el("span", undefined, row.detail);

    // Point the label at what it describes. A label centred across all five
    // columns sits under a harness that has nothing to do with the finding.
    const n = providers.length;
    if (row.ties.length) {
      const first = Math.min(...row.ties.map((t) => t.from));
      const last = Math.max(...row.ties.map((t) => t.to));
      inner.setAttribute(
        "style",
        `margin-left:${((first + 0.5) / n) * 100}%; width:${((last - first) / n) * 100}%;`,
      );
    } else {
      const flagged = row.cells.findIndex((c) => c.state === "bad" || c.state === "on");
      if (flagged >= 0) {
        inner.setAttribute("style", `margin-left:${(flagged / n) * 100}%; width:${(1 / n) * 100}%;`);
      }
    }

    // `.mrow` matches `.arow` column for column; without spacers the label
    // lands in the name column instead of over the cells.
    note.appendChild(el("div"));
    if (showLibrary) note.appendChild(el("div"));
    note.appendChild(inner);
    frag.appendChild(note);
  }

  return frag;
}

function renderOverview(data: Overview) {
  clear(main);
  // The title bar always says which world you are looking at. Project scope
  // gets its own colour, because the consequences of a write differ.
  setScopeLabel(data.project);

  if (data.providers.length === 0) {
    main.appendChild(
      el(
        "p",
        "empty",
        data.project
          ? "Nothing is set up in this project yet. Open a skill at user scope and add it here."
          : "No agent harnesses found on this machine.",
      ),
    );
    return;
  }

  const matrix = el("div", `matrix${data.library ? " has-library" : ""}`);

  // Column headers
  const head = el("div", "mrow");
  head.appendChild(el("div"));
  if (data.library) {
    const lib = el("button", "colname libname is-button", "Library");
    (lib as HTMLButtonElement).type = "button";
    lib.title =
      `~/.ditto/library — your copy, read by no harness.
It is the version a deploy uses when you do not pick one.
Click to manage it.`;
    lib.addEventListener("click", () => navigate(() => openLibrary()));
    head.appendChild(lib);
  }
  const names = el("div", "cells");
  names.style.gridTemplateColumns = `repeat(${data.providers.length}, 1fr)`;
  for (const provider of data.providers) {
    // "GitHub Copilot CLI" in a column 100px wide says less than "Copilot":
    // the suffix is how the harness is delivered, not which harness it is.
    const name = el("div", "colname", provider.name.replace(/^GitHub /, "").replace(/ (CLI|Code)$/, ""));
    name.title = provider.home ? `${provider.name}\n${provider.home}` : provider.name;
    names.appendChild(name);
  }
  head.appendChild(names);
  matrix.appendChild(head);
  matrix.appendChild(el("div", "rule"));

  const open = (name: string) => navigate(() => openDetail(name));

  const sections: [string, Row[]][] = [
    ["Skills", data.skills],
    ["Servers", data.servers],
  ];
  for (const [title, rows] of sections) {
    if (rows.length === 0) continue;
    matrix.appendChild(el("div", "group-label", title));
    for (const row of rows) {
      matrix.appendChild(renderRow(row, data.providers, open, data.library));
    }
  }

  const legend = el("div", "legend");
  const item = (swatch: HTMLElement, text: string) => {
    const wrap = el("div");
    const holder = el("span", "swatch");
    holder.appendChild(swatch);
    wrap.appendChild(holder);
    wrap.appendChild(document.createTextNode(text));
    return wrap;
  };
  legend.appendChild(item(el("i", "dot on"), "installed"));
  legend.appendChild(item(el("i", "dot off"), "not installed"));
  legend.appendChild(item(el("i", "dot bad"), "present but unreadable"));
  legend.appendChild(item(el("span", "bar shared"), "one shared file"));
  legend.appendChild(item(el("span", "bar"), "separate copies, identical"));
  legend.appendChild(item(el("span", "bar drift"), "copies have drifted"));
  legend.appendChild(item(el("span", "rowmark"), "in a deprecated location"));
  matrix.appendChild(legend);

  main.appendChild(matrix);
}

// ---------------------------------------------------------------------------
// Detail drawer
// ---------------------------------------------------------------------------

function closeDrawer() {
  drawer.hidden = true;
  scrim.hidden = true;
  screens.length = 0;
  clear(drawer);
}

scrim.addEventListener("click", closeDrawer);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !drawer.hidden) goBack();
});

/**
 * Screens the drawer has shown, newest last. Named `screens` rather than
 * `history` because that identifier is already the browser's.
 *
 * Each entry re-renders its screen, so Back steps out of a flow instead of
 * re-fetching the detail view and losing where the user had got to.
 */
const screens: Array<() => void | Promise<void>> = [];

function navigate(render: () => void | Promise<void>) {
  screens.push(render);
  void render();
}

function goBack() {
  screens.pop();
  const previous = screens[screens.length - 1];
  if (previous) void previous();
  else closeDrawer();
}

function drawerShell(name: string, status: string): HTMLElement {
  clear(drawer);
  drawer.hidden = false;
  scrim.hidden = false;

  const head = el("div", "drawer-head");
  if (screens.length > 1) {
    const back = el("button", "close back", "‹");
    back.type = "button";
    back.title = "Back";
    back.addEventListener("click", goBack);
    head.appendChild(back);
  }
  head.appendChild(el("span", "name", name));
  head.appendChild(el("span", `status${status === "drifted" ? " drifted" : ""}`, status));
  head.appendChild(el("span", "spacer"));
  const close = el("button", "close", "Close");
  close.type = "button";
  close.addEventListener("click", closeDrawer);
  head.appendChild(close);
  drawer.appendChild(head);

  const body = el("div", "drawer-body");
  drawer.appendChild(body);
  close.focus();
  return body;
}



async function openDetail(name: string) {
  const body = drawerShell(name, "reading…");
  try {
    const detail = await invoke<Detail>("detail", { name });
    clear(body);
    if (detail.kind === "server") {
      renderServer(body, detail);
    } else {
      renderSkill(body, detail);
    }
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
  }
}

function setStatus(text: string, warn = false) {
  const node = drawer.querySelector(".status") as HTMLElement;
  node.textContent = text;
  node.className = warn ? "status drifted" : "status";
}

/** A labelled value row, used by the server view. */
function field(label: string, value: string, mono = true): HTMLElement {
  const row = el("div", "field");
  row.appendChild(el("span", "flabel", label));
  row.appendChild(el("span", mono ? "fvalue mono" : "fvalue", value));
  return row;
}

function revealButton(path: string): HTMLElement {
  const btn = el("button", "btn", "Show in folder");
  btn.type = "button";
  // A button's label is the only thing that says what it does, so an error
  // must not be written into it permanently: the first failure used to leave
  // a control reading `C:\Users\...\library no longer e` — a message cut
  // off mid-word, in a button that then looked broken and never came back.
  // Say it briefly, keep the whole message in the tooltip, and restore.
  const label = btn.textContent ?? "";
  let restore = 0;
  btn.addEventListener("click", () => {
    void invoke("reveal", { path }).catch((e: unknown) => {
      btn.title = String(e);
      btn.classList.add("is-bad");
      btn.textContent = "Couldn't open it";
      window.clearTimeout(restore);
      restore = window.setTimeout(() => {
        btn.textContent = label;
        btn.classList.remove("is-bad");
      }, 2600);
    });
  });
  return btn;
}

function callout(title: string, message: string): HTMLElement {
  const box = el("div", "callout");
  box.appendChild(el("div", "callout-title", title));
  box.appendChild(el("div", "callout-body", message));
  return box;
}

function renderServer(body: HTMLElement, server: ServerDetail) {
  setStatus(server.transport + " · " + server.provider);

  const fields = el("div", "fields");
  if (server.command) fields.appendChild(field("command", server.command));
  if (server.url) fields.appendChild(field("url", server.url));
  fields.appendChild(
    field("scope", server.project ? server.scope + " · " + server.project : server.scope, false),
  );
  fields.appendChild(field("file", server.file));
  if (server.env_names.length) {
    fields.appendChild(field("environment", server.env_names.join(", ")));
  }
  if (server.extra_keys.length) {
    fields.appendChild(field("kept for this harness", server.extra_keys.join(", ")));
  }
  body.appendChild(fields);

  if (server.secret_names.length) {
    body.appendChild(
      callout(
        "Stores a credential in plaintext",
        server.secret_names.join(", ") +
          " is written in the clear in this file. Copying this server to another harness" +
          " puts another copy on disk.",
      ),
    );
  }

  const actions = el("div", "actions");
  actions.appendChild(
    el("span", "count", "Copies this server to harnesses that do not have it."),
  );
  actions.appendChild(revealButton(server.file));
  const go = el("button", "btn primary", "Preview changes…");
  go.type = "button";
  go.addEventListener("click", () => navigate(() => openPlan(server.name)));
  actions.appendChild(go);
  body.appendChild(actions);
}

function renderSkill(body: HTMLElement, detail: SkillDetail) {
  const copies = detail.copies.length;
  setStatus(
    detail.version_count > 1
      ? copies + " copies · " + detail.version_count + " versions"
      : copies === 1
        ? "1 copy"
        : copies + " copies · all the same",
    detail.version_count > 1,
  );

  // A directory with no readable SKILL.md is not something to deploy, it is
  // something to look at. Offering a deploy button here was a dead end.
  if (detail.unusable) {
    body.appendChild(
      callout(
        "No harness can load this",
        "The directory exists but has no readable SKILL.md, so every harness ignores it." +
          " Either add a SKILL.md or delete the directory.",
      ),
    );
    for (const copy of detail.copies) {
      const row = el("div", "paperhead");
      row.appendChild(el("span", undefined, copy.path));
      if (copy.deprecated) row.appendChild(el("span", "flag", "deprecated location"));
      body.appendChild(row);
    }
    const actions = el("div", "actions");
    actions.appendChild(el("span", "count", ""));
    if (detail.copies[0]) actions.appendChild(revealButton(detail.copies[0].path));
    body.appendChild(actions);
    return;
  }

  if (detail.comparison) {
    body.appendChild(el("div", "sectiontitle", "How the versions differ"));
    body.appendChild(renderComparison(detail.comparison));
  }

  body.appendChild(el("div", "sectiontitle", copies === 1 ? "Installed at" : "Copies"));
  for (const copy of detail.copies) {
    const row = el("div", "paperhead");
    row.appendChild(el("span", undefined, copy.path));
    row.appendChild(el("span", "who", holder(copy)));
    row.appendChild(el("span", "hash", copy.hash));
    if (copy.changed) row.appendChild(el("span", "muted when", `changed ${copy.changed}`));
    if (copy.deprecated) row.appendChild(el("span", "flag", "deprecated location"));
    if (copy.path === detail.source && detail.version_count > 1) {
      row.appendChild(el("span", "flag source", "this version wins"));
    }
    body.appendChild(row);
  }

  const missing = detail.targets.filter((t) => !t.has_it).length;
  const drifted = detail.version_count > 1;

  const actions = el("div", "actions");
  actions.appendChild(
    el(
      "span",
      "count",
      drifted
        ? "Pick which version every copy should match."
        : missing === 0
          ? "Every harness already has this."
          : "Choose where to install it.",
    ),
  );
  if (detail.source) actions.appendChild(revealButton(detail.source));

  // The label names the intent and says how much it touches, so the button is
  // predictable before it is pressed.
  if (drifted || missing > 0) {
    const label = drifted
      ? "Resolve differences…"
      : `Add to ${missing} harness${missing === 1 ? "" : "es"}…`;
    const go = el("button", "btn primary", label);
    go.type = "button";
    go.addEventListener("click", () => navigate(() => openConfigure(detail)));
    actions.appendChild(go);
  }

  // Offered only where it can do something: the library has to be on, and at
  // project scope it is not a row, so there is nothing to adopt into from here.
  if (detail.library_enabled && !detail.unusable && !current?.project) {
    const label = detail.in_library ? "Update library copy…" : "Add to library…";
    const adopt = el("button", "btn", label);
    adopt.type = "button";
    adopt.addEventListener("click", () => navigate(() => openImport(detail.name)));
    actions.appendChild(adopt);
  }

  // The operation that makes project scope worth having (T-6.5): the same skill,
  // moved between "everywhere I work" and "this repository, for my team".
  if (!detail.unusable) {
    const inProject = Boolean(current?.project);
    const transfer = el(
      "button",
      "btn",
      inProject ? "Also add to your account…" : "Also add to a project…",
    );
    transfer.type = "button";
    transfer.addEventListener("click", () => {
      if (inProject) {
        navigate(() => openTransfer(detail.name, false));
      } else {
        navigate(() => openScopePicker((_project) => void openTransfer(detail.name, true)));
      }
    });
    actions.appendChild(transfer);
  }

  body.appendChild(actions);
}

// ---------------------------------------------------------------------------
// Configure — choose the version, choose the targets (T-5.17, T-5.18)
// ---------------------------------------------------------------------------

/**
 * One screen for both jobs. The version picker only appears when copies
 * actually disagree, so an install never asks a question it does not need to.
 *
 * The write summary is produced by re-planning on every change rather than by
 * reimplementing the planner here — so the count the user sees is the count the
 * engine will perform.
 */
async function openConfigure(detail: SkillDetail) {
  const drifted = detail.version_count > 1;
  const body = drawerShell(detail.name, drifted ? "choose a version" : "choose targets");

  let source: string | undefined = detail.versions.find((v) => v.is_default)?.source;
  // Resolving touches everything that has a copy; installing targets the gaps.
  const selected = new Set(
    detail.targets.filter((t) => (drifted ? true : !t.has_it)).map((t) => t.id),
  );

  if (drifted) {
    body.appendChild(el("div", "sectiontitle", "Which version should win?"));
    const list = el("div", "choices");
    for (const version of detail.versions) {
      const row = el("label", "choice");
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "version";
      radio.value = version.source;
      radio.checked = version.source === source;
      radio.addEventListener("change", () => {
        source = version.source;
        updateNotes();
        void recount();
      });
      row.appendChild(radio);

      const text = el("div", "choice-body");
      const top = el("div", "choice-top");
      top.appendChild(el("span", "hash", version.hash));
      top.appendChild(el("span", "who", holder(version) || "not in a harness"));
      top.appendChild(
        el("span", "muted", `${version.paths.length} cop${version.paths.length === 1 ? "y" : "ies"}`),
      );
      // Recency is here to inform the choice, not to make it: a deploy writes
      // its destination, so the newest file is often the copy rather than the
      // original. The tooltip says so rather than leaving it to be assumed.
      if (version.changed) {
        const when = el("span", "muted when", `changed ${version.changed}`);
        when.title =
          `When this version last changed on disk.
Copying a skill updates the copy, so the newest file is not always the original.`;
        top.appendChild(when);
      }
      text.appendChild(top);
      text.appendChild(el("div", "choice-desc", version.description || "(no description)"));
      text.appendChild(el("div", "choice-path", version.source));
      row.appendChild(text);
      list.appendChild(row);
    }
    body.appendChild(list);
  }

  body.appendChild(el("div", "sectiontitle", drifted ? "Apply to" : "Install on"));
  const targets = el("div", "choices");
  const notes = new Map<string, HTMLElement>();
  for (const target of detail.targets) {
    const row = el("label", "choice");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = selected.has(target.id);
    box.addEventListener("change", () => {
      if (box.checked) selected.add(target.id);
      else selected.delete(target.id);
      void recount();
    });
    row.appendChild(box);
    const text = el("div", "choice-body");
    const top = el("div", "choice-top");
    top.appendChild(el("span", undefined, target.name));
    const note = el("span", "muted");
    top.appendChild(note);
    notes.set(target.id, note);
    text.appendChild(top);
    row.appendChild(text);
    targets.appendChild(row);
  }
  body.appendChild(targets);

  /**
   * Say what happens to each harness *given the chosen version*, not merely
   * whether it has some copy.
   *
   * "already has it" was true of every harness in a resolve flow — which is
   * exactly why they disagreed — so it read as "nothing to do" beside a button
   * about to rewrite three of them.
   */
  function updateNotes() {
    const winner = detail.versions.find((v) => v.source === source);
    for (const target of detail.targets) {
      const note = notes.get(target.id);
      if (!note) continue;
      const matches = winner ? winner.providers.includes(target.id) : false;
      if (matches) {
        note.textContent = "already matches";
        note.className = "muted";
      } else if (target.has_it) {
        note.textContent = "will be replaced";
        note.className = "muted change";
      } else {
        note.textContent = "will be added";
        note.className = "muted add";
      }
    }
  }

  const summary = el("div", "summary", "…");
  body.appendChild(summary);

  const actions = el("div", "actions");
  actions.appendChild(el("span", "count", ""));
  const cancel = el("button", "btn", "Back");
  cancel.type = "button";
  cancel.addEventListener("click", goBack);
  const go = el("button", "btn primary", "Preview changes…");
  go.type = "button";
  go.addEventListener("click", () =>
    navigate(() => openPlan(detail.name, [...selected], source)),
  );
  actions.appendChild(cancel);
  actions.appendChild(go);
  body.appendChild(actions);

  /** Ask the engine what this selection costs, and say it in plain terms. */
  async function recount() {
    if (selected.size === 0) {
      summary.textContent = "Nothing selected.";
      go.disabled = true;
      return;
    }
    go.disabled = false;
    summary.textContent = "…";
    try {
      const plan = await invoke<PlanView>("plan_deploy", {
        name: detail.name,
        targets: [...selected],
        source,
      });
      const writes = plan.steps.length;
      const replacing = plan.steps.filter((s) => s.warnings.length > 0).length;
      if (writes === 0) {
        summary.textContent = "Nothing to do — every harness you picked already matches.";
      } else {
        const served = plan.served.length;
        // Ticked harnesses that need no write already agree with the chosen
        // version. Saying so stops the count looking like it lost some.
        const untouched = selected.size - served;
        const parts = [`${writes} write${writes === 1 ? "" : "s"}`];
        parts.push(`${served} harness${served === 1 ? "" : "es"} change`);
        if (untouched > 0) parts.push(`${untouched} already match`);
        let text = parts.join(" · ");
        // The shared-directory saving, stated at the moment it is earned.
        if (writes < served) text += " — one file is shared";
        if (replacing > 0) {
          text += `\n${replacing} of these replaces a copy that differs. Backups go to ~/.ditto/backups.`;
        }
        summary.textContent = text;
      }
    } catch (error) {
      summary.textContent = String(error);
    }
  }

  updateNotes();
  void recount();
}

// ---------------------------------------------------------------------------
// Deploy
// ---------------------------------------------------------------------------

/**
 * A *change*: something will be written. `+` and `-` are right here, because
 * there genuinely is a before and an after.
 */
function renderChange(lines: DiffLine[]): HTMLElement {
  const diff = el("div", "diff");
  for (const line of lines) {
    const sign = line.kind === "add" ? "+ " : line.kind === "del" ? "- " : "  ";
    diff.appendChild(el("div", line.kind, sign + line.text));
  }
  return diff;
}

/**
 * A *comparison*: two versions that both exist. Nothing is being changed by
 * looking at them, so `+`/`-` would be a lie — a `-` line reads as "about to be
 * deleted" when it is simply what one harness has today.
 *
 * Markers name the side instead, the header says whose each one is, and the
 * tints are achromatic so neither reads as the bad one. Violet and amber stay
 * reserved for routing and caution.
 */
function renderComparison(comparison: Comparison): HTMLElement {
  const wrap = el("div", "cmp");

  const key = el("div", "cmp-key");
  for (const [mark, side] of [["◀", comparison.left], ["▶", comparison.right]] as const) {
    const row = el("div", "cmp-key-row");
    row.appendChild(el("span", "cmp-mark", mark));
    row.appendChild(el("span", "hash", side.hash));
    row.appendChild(el("span", "who", holder(side)));
    if (side.changed) row.appendChild(el("span", "muted when", `changed ${side.changed}`));
    row.appendChild(el("span", "cmp-path", side.path));
    key.appendChild(row);
  }
  wrap.appendChild(key);

  const body = el("div", "diff cmp-body");
  for (const line of comparison.lines) {
    const side = line.kind === "del" ? "a" : line.kind === "add" ? "b" : "same";
    const mark = side === "a" ? "◀ " : side === "b" ? "▶ " : "  ";
    body.appendChild(el("div", "cmp-line " + side, mark + line.text));
  }
  wrap.appendChild(body);
  return wrap;
}

function renderStep(step: Step): HTMLElement {
  const wrap = el("div", "write");
  const removing = step.kind.endsWith("remove");
  const shared = step.providers.length > 1;
  wrap.appendChild(
    el("div", `spine${removing ? " danger" : shared ? "" : " solo"}`),
  );

  const body = el("div", "body");
  body.appendChild(el("div", "path", step.target));

  const serves = el("div", "serves");
  serves.appendChild(document.createTextNode("serves "));
  step.providers.forEach((provider, index) => {
    if (index > 0) serves.appendChild(document.createTextNode(" · "));
    serves.appendChild(el("b", undefined, provider));
  });
  body.appendChild(serves);
  body.appendChild(el("div", "why", step.reason));

  if (step.dropped.length) {
    body.appendChild(
      el("div", "note", `dropped, not supported here: ${step.dropped.join(", ")}`),
    );
  }
  for (const warning of step.warnings) body.appendChild(el("div", "note", warning));

  // A write inside a working tree is a write a teammate will see. We report the
  // repository, not whether it is ignored — .gitignore has enough corner cases
  // that a partial answer would be worse than an honest one.
  if (step.git) {
    body.appendChild(
      el("div", "git", `inside the repository at ${step.git} — it will show up in git status`),
    );
  }

  if (step.diff.length) body.appendChild(renderChange(step.diff));

  wrap.appendChild(body);
  return wrap;
}

async function openPlan(name: string, chosen?: string[], source?: string) {
  const targets = chosen ?? (current?.providers ?? []).map((p) => p.id);
  const body = drawerShell(name, "planning…");

  try {
    const plan = await invoke<PlanView>("plan_deploy", { name, targets, source });
    clear(body);
    setStatus(
      plan.steps.length === 0
        ? "nothing to do"
        : `${plan.steps.length} write${plan.steps.length === 1 ? "" : "s"} · ${plan.served.length} served`,
    );

    if (plan.steps.length === 0 && plan.refusals.length === 0) {
      body.appendChild(el("p", "empty", `${name} is already everywhere it can go.`));
      return;
    }

    if (plan.steps.length) {
      body.appendChild(el("div", "sectiontitle", "Will write"));
      const set = el("div", "writeset");
      for (const step of plan.steps) set.appendChild(renderStep(step));
      body.appendChild(set);
    }

    if (plan.refusals.length) {
      body.appendChild(el("div", "sectiontitle", "Skipped"));
      for (const refusal of plan.refusals) {
        const row = el("div", "refusal");
        row.appendChild(el("div", "spine"));
        const inner = el("div");
        inner.appendChild(el("div", "who", refusal.provider));
        inner.appendChild(el("div", "why", refusal.reason));
        row.appendChild(inner);
        body.appendChild(row);
      }
    }

    if (plan.steps.length === 0) return;

    const actions = el("div", "actions");
    actions.appendChild(
      el("span", "count", "Backups go to ~/.ditto/backups before anything is replaced."),
    );
    const cancel = el("button", "btn", "Back");
    cancel.type = "button";
    cancel.addEventListener("click", goBack);
    const go = el("button", "btn primary",
      `Write ${plan.steps.length} file${plan.steps.length === 1 ? "" : "s"}`);
    go.type = "button";
    go.addEventListener("click", () => {
      go.disabled = true;
      cancel.disabled = true;
      go.textContent = "Writing…";
      void runPlan(name, targets, source);
    });
    actions.appendChild(cancel);
    actions.appendChild(go);
    body.appendChild(actions);
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
  }
}

async function runPlan(name: string, targets: string[], source?: string) {
  try {
    const outcomes = await invoke<Outcome[]>("apply", { name, targets, source });
    const body = drawerShell(name, "done");
    const failed = outcomes.filter((o) => !o.ok).length;
    setStatus(failed === 0 ? `${outcomes.length} written` : `${failed} failed`, failed > 0);

    body.appendChild(el("div", "sectiontitle", "Result"));
    for (const outcome of outcomes) {
      const row = el("div", "outcome");
      row.appendChild(el("div", `badge ${outcome.ok ? "ok" : "fail"}`, outcome.ok ? "written" : "failed"));
      const inner = el("div");
      inner.appendChild(el("div", "what", outcome.target));
      inner.appendChild(el("div", "msg", outcome.message));
      row.appendChild(inner);
      body.appendChild(row);
    }

    const actions = el("div", "actions");
    actions.appendChild(el("span", "count", ""));
    const done = el("button", "btn primary", "Done");
    done.type = "button";
    done.addEventListener("click", () => {
      closeDrawer();
      void refresh();
    });
    actions.appendChild(done);
    body.appendChild(actions);
  } catch (error) {
    const body = drawerShell(name, "failed");
    body.appendChild(el("pre", "error", String(error)));
  }
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

type FindingAction =
  | { kind: "open"; name: string }
  | { kind: "reveal"; path: string };
type Finding = {
  severity: "security" | "warning" | "info";
  provider: string | null;
  message: string;
  suggestion: string | null;
  action: FindingAction | null;
};

async function renderDoctor() {
  clear(main);
  main.appendChild(el("p", "loading", "Checking…"));
  try {
    const findings = await invoke<Finding[]>("doctor");
    clear(main);
    if (findings.length === 0) {
      main.appendChild(el("p", "empty", "No problems found."));
      return;
    }
    const list = el("div", "findings");
    for (const finding of findings) {
      const row = el("div", "finding");
      const label = finding.severity === "security" ? "SECURITY" : finding.provider ?? "";
      row.appendChild(el("div", `sev${finding.severity === "security" ? " security" : ""}`, label));
      const inner = el("div");
      inner.appendChild(el("div", "msg", finding.message));
      if (finding.suggestion) inner.appendChild(el("div", "fix", finding.suggestion));

      // Every suggestion already names an action. Put it on a button rather
      // than leaving the reader to go and find the thing themselves.
      if (finding.action) {
        const bar = el("div", "finding-actions");
        if (finding.action.kind === "open") {
          const name = finding.action.name;
          const go = el("button", "btn", "Open");
          go.type = "button";
          go.addEventListener("click", () => navigate(() => openDetail(name)));
          bar.appendChild(go);
        } else {
          bar.appendChild(revealButton(finding.action.path));
        }
        inner.appendChild(bar);
      }
      row.appendChild(inner);
      list.appendChild(row);
    }
    main.appendChild(list);
  } catch (error) {
    clear(main);
    main.appendChild(el("pre", "error", String(error)));
  }
}

// ---------------------------------------------------------------------------
// Scope picker (T-6.4)
// ---------------------------------------------------------------------------

/**
 * Projects come from what the harnesses have already recorded, so the list is
 * the places the user actually works rather than a directory tree to hunt
 * through. "User scope" is first because it is the default and the way back.
 */
async function openScopePicker(then?: (project: Project) => void) {
  const body = drawerShell(then ? "Choose a project" : "Scope", "loading…");
  let projects: Project[] = [];
  try {
    projects = await invoke<Project[]>("projects");
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
    return;
  }

  clear(body);
  setStatus(projects.length === 1 ? "1 project" : `${projects.length} projects`);
  body.appendChild(
    el(
      "p",
      "lede",
      "User scope covers every project on this machine. Project scope writes into" +
        " the repository, where your team sees it.",
    ),
  );

  const list = el("div", "picker");
  const active = current?.project?.path ?? null;
  // Declared up here because the forget handler below closes over it.
  const problem = el("div", "addnote");
  // Re-opening rebuilds the drawer in place. `navigate` is deliberately not
  // used: adding a project is not a new screen, and pushing one would put a
  // back button over the picker you are still standing in.
  const redraw = () => openScopePicker(then);

  const choose = async (project: Project | null) => {
    await invoke<Project | null>("set_project", { path: project?.path ?? null });
    if (project && then) {
      // Switching scope changed what the window is showing, so refresh behind
      // the drawer before the transfer preview replaces it.
      await refresh();
      then(project);
      return;
    }
    closeDrawer();
    await refresh();
  };

  // Not offered when picking a destination for a transfer — "your account" is
  // where the thing already is.
  if (!then) {
    const account = el("button", `pick${active === null ? " is-on" : ""}`);
    account.type = "button";
    account.appendChild(el("span", undefined, "User scope"));
    account.appendChild(el("span", "where", "user scope"));
    account.addEventListener("click", () => void choose(null));
    list.appendChild(account);
  }

  for (const project of projects) {
    const button = el("button", `pick${active === project.path ? " is-on" : ""}`);
    button.type = "button";
    button.appendChild(el("span", undefined, project.name));

    const tags = el("span", "tags");
    if (project.is_git) tags.appendChild(el("span", "tag", "git"));
    // Markers say "this repo is already set up for something", which is the
    // question you actually have when picking one out of twenty.
    for (const marker of project.markers.slice(0, 3)) {
      tags.appendChild(el("span", "tag", marker));
    }
    // No harness has been opened here, so there is nothing in it yet — worth
    // saying, because an empty matrix afterwards is otherwise puzzling.
    if (project.sources.length === 0) tags.appendChild(el("span", "tag", "not used yet"));
    button.appendChild(tags);

    button.appendChild(el("span", "where", project.path));
    button.addEventListener("click", () => void choose(project));

    // Only what you added can be removed. A harness's record lives in that
    // harness's own config, so taking it off here would make the row vanish
    // and then reappear on the next scan.
    if (project.added) {
      const row = el("div", "pickrow");
      row.appendChild(button);
      const forget = el("button", "forget", "×");
      forget.type = "button";
      forget.title = `Take ${project.name} off your list. The directory is not touched.`;
      forget.addEventListener("click", (event) => {
        event.stopPropagation();
        void (async () => {
          try {
            await invoke<Project[]>("forget_project", { path: project.path });
            await redraw();
          } catch (error) {
            problem.textContent = String(error);
          }
        })();
      });
      row.appendChild(forget);
      list.appendChild(row);
    } else {
      list.appendChild(button);
    }
  }

  body.appendChild(list);

  if (projects.length === 0) {
    body.appendChild(
      el(
        "p",
        "empty",
        "No harness has recorded a project yet, and you have not added one.",
      ),
    );
  }

  // --- add a project --------------------------------------------------------
  //
  // Discovery reads what a harness recorded, which cannot include a repo you
  // have never opened an agent in — and that is exactly the repo you would open
  // this picker to set up.
  body.appendChild(el("div", "sectiontitle", "Add a project"));
  const adder = el("div", "adder");
  const input = document.createElement("input");
  input.type = "text";
  input.className = "pathinput";
  input.placeholder = "browse, or paste a folder path";
  input.spellcheck = false;
  const browseBtn = el("button", "btn", "Browse…");
  browseBtn.type = "button";
  const addBtn = el("button", "btn", "Add");
  addBtn.type = "button";

  const submit = () => {
    // Explorer's "Copy as path" wraps the path in quotes, and pasting that
    // verbatim is the likeliest way to use this field.
    const path = input.value.trim().replace(/^"|"$/g, "");
    if (!path) return;
    addBtn.disabled = true;
    problem.textContent = "";
    void (async () => {
      try {
        await invoke<Project[]>("add_project", { path });
        await redraw();
      } catch (error) {
        problem.textContent = String(error);
        addBtn.disabled = false;
      }
    })();
  };
  // Picking a folder *is* the decision, so it adds straight away rather than
  // filling the field and waiting for a second click on Add. Cancelling leaves
  // everything untouched.
  browseBtn.addEventListener("click", () => {
    browseBtn.disabled = true;
    problem.textContent = "";
    void (async () => {
      try {
        const picked = await invoke<string | null>("pick_folder");
        if (picked) {
          input.value = picked;
          submit();
        }
      } catch (error) {
        problem.textContent = String(error);
      } finally {
        browseBtn.disabled = false;
      }
    })();
  });

  addBtn.addEventListener("click", submit);
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") submit();
  });

  adder.appendChild(input);
  adder.appendChild(browseBtn);
  adder.appendChild(addBtn);
  body.appendChild(adder);
  body.appendChild(problem);
  body.appendChild(
    el(
      "p",
      "hint",
      "Kept in your Ditto settings, not written into any harness's own project list.",
    ),
  );
}

/**
 * Copy an artifact between your account and the selected project.
 *
 * The preview is the same write list a deploy shows, because it is the same
 * planner — a transfer is a deploy whose source happens to sit in the other
 * scope. Nothing is written until Apply.
 */
async function openTransfer(name: string, toProject: boolean) {
  const where = toProject ? current?.project?.name ?? "this project" : "your account";
  const body = drawerShell(name, "planning…");
  let plan: PlanView;
  try {
    plan = await invoke<PlanView>("plan_transfer", { name, toProject, targets: [] });
  } catch (error) {
    clear(body);
    setStatus("cannot transfer", true);
    body.appendChild(el("pre", "error", String(error)));
    return;
  }

  clear(body);
  setStatus(
    plan.steps.length === 0
      ? "nothing to do"
      : `${plan.steps.length} write${plan.steps.length === 1 ? "" : "s"} · ${plan.served.length} served`,
  );
  body.appendChild(el("p", "lede", `Copying ${name} into ${where}.`));

  if (plan.steps.length === 0) {
    body.appendChild(el("p", "empty", `${name} is already in ${where}.`));
    return;
  }

  body.appendChild(el("div", "sectiontitle", "Will write"));
  const set = el("div", "writeset");
  for (const step of plan.steps) set.appendChild(renderStep(step));
  body.appendChild(set);

  const actions = el("div", "actions");
  actions.appendChild(el("span", "count", `${plan.served.length} harnesses will see it.`));
  const go = el("button", "btn primary", "Copy it");
  go.type = "button";
  go.addEventListener("click", () => {
    go.disabled = true;
    void (async () => {
      try {
        const outcomes = await invoke<Outcome[]>("apply_transfer", {
          name,
          toProject,
          targets: [],
        });
        clear(body);
        setStatus(outcomes.every((o) => o.ok) ? "done" : "partly done", outcomes.some((o) => !o.ok));
        for (const outcome of outcomes) {
          const row = el("div", outcome.ok ? "note" : "error");
          row.textContent = `${outcome.target} — ${outcome.message}`;
          body.appendChild(row);
        }
        await refresh();
      } catch (error) {
        body.appendChild(el("pre", "error", String(error)));
      }
    })();
  });
  actions.appendChild(go);
  body.appendChild(actions);
}

/**
 * Adopt a copy into the library (Q-7).
 *
 * Deliberately its own screen rather than another checkbox on the configure
 * flow: importing decides *which copy is canonical*, while deploying decides
 * *where a copy goes*. Folding them together would make every deploy an
 * import, and the library would fill up with copies nobody chose.
 */
async function openImport(name: string, source?: string) {
  const body = drawerShell(name, "planning…");
  let plan: PlanView;
  try {
    plan = await invoke<PlanView>("plan_import", { name, source: source ?? null });
  } catch (error) {
    clear(body);
    setStatus("cannot import", true);
    body.appendChild(el("pre", "error", String(error)));
    return;
  }

  clear(body);
  body.appendChild(
    el(
      "p",
      "lede",
      `Copying ${name} into your library. A deploy uses the library's copy` +
        ` unless you pick a different one, so this is how you say which version is yours.`,
    ),
  );

  if (plan.steps.length === 0) {
    setStatus("nothing to do");
    for (const refusal of plan.refusals) {
      body.appendChild(el("div", "note", refusal.reason));
    }
    if (plan.refusals.length === 0) {
      body.appendChild(el("p", "empty", "Your library already holds this."));
    }
    return;
  }

  setStatus("1 write");
  body.appendChild(el("div", "sectiontitle", "Will write"));
  const set = el("div", "writeset");
  for (const step of plan.steps) set.appendChild(renderStep(step));
  body.appendChild(set);

  const actions = el("div", "actions");
  actions.appendChild(el("span", "count", "Harness config is not touched."));
  const go = el("button", "btn primary", "Add to library");
  go.type = "button";
  go.addEventListener("click", () => {
    go.disabled = true;
    void (async () => {
      try {
        const outcomes = await invoke<Outcome[]>("apply_import", {
          name,
          source: source ?? null,
        });
        clear(body);
        setStatus(outcomes.every((o) => o.ok) ? "done" : "partly done", outcomes.some((o) => !o.ok));
        for (const outcome of outcomes) {
          const row = el("div", outcome.ok ? "note" : "error");
          row.textContent = `${outcome.target} — ${outcome.message}`;
          body.appendChild(row);
        }
        await refresh();
      } catch (error) {
        body.appendChild(el("pre", "error", String(error)));
      }
    })();
  });
  actions.appendChild(go);
  body.appendChild(actions);
}

/**
 * What the library is, and the switch for it.
 *
 * The setting lived only in `~/.ditto/config.toml` when the library
 * shipped, which made it something you had to already know about to find. It
 * hangs off the column header because that is the thing it controls.
 */
async function openLibrary() {
  const body = drawerShell("Library", "loading…");
  let info: LibraryInfo;
  try {
    info = await invoke<LibraryInfo>("library_setting");
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
    return;
  }

  clear(body);
  setStatus(info.enabled ? "on" : "off");
  body.appendChild(
    el(
      "p",
      "lede",
      "Your own copy of a skill, read by no harness. When several copies" +
        " disagree, the library's is the one a deploy uses unless you pick" +
        " another — which is the question it exists to answer.",
    ),
  );

  if (info.path) {
    const where = el("div", "paperhead");
    where.appendChild(el("span", undefined, info.path));
    body.appendChild(where);
  }

  // A switch rather than a checkbox, and deliberately not the `.choice` row
  // the deploy flows use: those are a list of things being ticked, this is one
  // mode the whole window is in. The checkbox underneath stays, hidden, so the
  // control is still a real form control for the keyboard and for the reader.
  const row = el("label", "switchrow");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.className = "switchbox";
  box.checked = info.enabled;
  row.appendChild(box);
  const track = el("span", "track");
  track.appendChild(el("span", "knob"));
  row.appendChild(track);
  const text = el("div", "choice-body");
  text.appendChild(el("div", "choice-top", "Use the library"));
  text.appendChild(
    el(
      "div",
      "choice-desc",
      "Switched off, that directory is simply not scanned and a deploy falls" +
        " back to picking the first copy it can read. Nothing else changes," +
        " and nothing in the library is deleted.",
    ),
  );
  row.appendChild(text);
  body.appendChild(row);

  const note = el("div", "addnote");
  box.addEventListener("change", () => {
    box.disabled = true;
    void (async () => {
      try {
        const next = await invoke<LibraryInfo>("set_library", { enabled: box.checked });
        setStatus(next.enabled ? "on" : "off");
        note.textContent = "Saved.";
        await refresh();
      } catch (error) {
        box.checked = !box.checked;
        note.textContent = String(error);
      } finally {
        box.disabled = false;
      }
    })();
  });
  body.appendChild(note);

  if (info.settings_path) {
    body.appendChild(el("p", "hint", `Saved in ${info.settings_path}`));
  }

  const actions = el("div", "actions");
  actions.appendChild(
    el("span", "count", "Add a skill from its drawer — a deploy never fills this for you."),
  );
  if (info.path && info.exists) actions.appendChild(revealButton(info.path));
  body.appendChild(actions);
}

/**
 * Draw the scope switcher.
 *
 * It used to be the text `name · project` in a thin border, which read as a
 * status line — people did not find it when they wanted to change project.
 * Three things make it legible as a control: an icon saying what kind of place
 * this is, the place's name, and a caret saying the thing opens.
 */
function setScopeLabel(project: Project | null) {
  clear(scopeLabel);
  scopeLabel.appendChild(icon(project ? "folder" : "person", "scope-icon"));
  // "User scope" rather than "Your account": every other surface — the CLI,
  // the doctor, the plan previews — calls it a scope, and a switcher that
  // renames the thing it switches makes the two read as different ideas.
  scopeLabel.appendChild(el("span", "scope-name", project ? project.name : "User scope"));
  scopeLabel.appendChild(icon("caret", "caret"));
  scopeLabel.classList.toggle("is-project", Boolean(project));
  scopeLabel.title = project
    ? `Project scope — writes land in ${project.path}\nClick to switch project`
    : "User scope — writes land in your home directory\nClick to work in a project";
}

scopeLabel.addEventListener("click", () => {
  screens.length = 0;
  void openScopePicker();
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

let view: "overview" | "doctor" = "overview";

tabs.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest(".tab") as HTMLElement | null;
  if (!button) return;
  const next = button.dataset.view as typeof view;
  if (next === view) return;
  view = next;
  for (const tab of tabs.querySelectorAll(".tab")) tab.classList.remove("is-on");
  button.classList.add("is-on");
  void refresh();
});

/**
 * Reads in flight. A counter rather than a boolean because a drawer can close
 * into a refresh while one is already running, and the second one finishing
 * must not declare the first one done.
 */
let reading = 0;

async function refresh() {
  reading += 1;
  refreshButton.classList.add("is-busy");
  try {
    if (view === "doctor") return await renderDoctor();
    clear(main);
    main.appendChild(el("p", "loading", "Reading your harnesses…"));
    try {
      current = await invoke<Overview>("overview");
      renderOverview(current);
    } catch (error) {
      clear(main);
      main.appendChild(el("pre", "error", String(error)));
    }
  } finally {
    reading -= 1;
    if (reading === 0) refreshButton.classList.remove("is-busy");
  }
}

// The machine changes underneath us — another window, a harness updating
// itself, our own writes. Nothing here polls, so offer an explicit re-read.
// `clear` first: the markup carries the word "Refresh" so the button still
// says something if this script never runs, but once it does the text has to
// go — appending alone left the label and the icon side by side.
clear(refreshButton);
refreshButton.appendChild(icon("refresh"));
refreshButton.addEventListener("click", () => void refresh());

// --- theme ----------------------------------------------------------------
// The value already sits on <html>, written by the inline script in index.html
// before the first paint. All this does is flip it and remember the choice.
type Theme = "dark" | "light";
const THEME_KEY = "ditto-theme";

function drawTheme() {
  const dark = document.documentElement.dataset.theme !== "light";
  clear(themeButton);
  // The icon is the theme you would get by pressing it, not the one you are
  // in: a button showing the current state invites you to press it to keep it.
  themeButton.appendChild(icon(dark ? "sun" : "moon"));
  themeButton.title = dark ? "Switch to the light theme" : "Switch to the dark theme";
}

themeButton.addEventListener("click", () => {
  const next: Theme = document.documentElement.dataset.theme === "light" ? "dark" : "light";
  document.documentElement.dataset.theme = next;
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {
    // Storage can be unavailable or full. The theme still changes; it just
    // will not survive a restart, which is better than refusing to change.
  }
  drawTheme();
});

drawTheme();

void refresh();
