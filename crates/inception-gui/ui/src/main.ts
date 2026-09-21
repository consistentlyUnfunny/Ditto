/**
 * inception — desktop UI.
 *
 * Every decision lives in the Rust engine; this file asks questions and draws
 * the answers. In particular the deploy preview is rendered from the same plan
 * the apply call executes, so what you see is what gets written.
 */

// ---------------------------------------------------------------------------
// Types — mirrors of crates/inception-gui/src/dto.rs
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
};
type Overview = { scope: string; providers: Provider[]; skills: Row[]; servers: Row[] };

type Copy = {
  path: string; providers: string[]; description: string; body: string;
  hash: string; deprecated: boolean; readable: boolean;
};
type Side = { path: string; providers: string[]; hash: string };
type Comparison = { left: Side; right: Side; lines: DiffLine[] };
type Version = {
  hash: string;
  description: string;
  providers: string[];
  paths: string[];
  source: string;
  is_default: boolean;
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
const scopeLabel = document.getElementById("scope") as HTMLElement;
const tabs = document.getElementById("tabs") as HTMLElement;
const refreshButton = document.getElementById("refresh") as HTMLButtonElement;

let current: Overview | null = null;

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

function renderRow(row: Row, providers: Provider[], onOpen: (name: string) => void) {
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

    // `.mrow` is a two-column grid; without a spacer the label lands in the
    // name column instead of over the cells.
    note.appendChild(el("div"));
    note.appendChild(inner);
    frag.appendChild(note);
  }

  return frag;
}

function renderOverview(data: Overview) {
  clear(main);
  scopeLabel.textContent = data.scope;

  if (data.providers.length === 0) {
    main.appendChild(el("p", "empty", "No agent harnesses found on this machine."));
    return;
  }

  const matrix = el("div", "matrix");

  // Column headers
  const head = el("div", "mrow");
  head.appendChild(el("div"));
  const names = el("div", "cells");
  names.style.gridTemplateColumns = `repeat(${data.providers.length}, 1fr)`;
  for (const provider of data.providers) {
    const name = el("div", "colname", provider.name.replace(/ (CLI|Code)$/, ""));
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
    for (const row of rows) matrix.appendChild(renderRow(row, data.providers, open));
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
  btn.addEventListener("click", () => {
    void invoke("reveal", { path }).catch((e) => {
      btn.textContent = String(e).slice(0, 44);
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
    row.appendChild(el("span", "who", copy.providers.join(", ")));
    row.appendChild(el("span", "hash", copy.hash));
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
      top.appendChild(el("span", "who", version.providers.join(", ")));
      top.appendChild(
        el("span", "muted", `${version.paths.length} cop${version.paths.length === 1 ? "y" : "ies"}`),
      );
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
          text += `\n${replacing} of these replaces a copy that differs. Backups go to ~/.inception/backups.`;
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
    row.appendChild(el("span", "who", side.providers.join(", ")));
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
      el("span", "count", "Backups go to ~/.inception/backups before anything is replaced."),
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
      // The machine changes underneath us — another window, a harness updating
// itself, our own writes. Nothing here polls, so offer an explicit re-read.
refreshButton.addEventListener("click", () => void refresh());

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
  // The machine changes underneath us — another window, a harness updating
// itself, our own writes. Nothing here polls, so offer an explicit re-read.
refreshButton.addEventListener("click", () => void refresh());

void refresh();
});

async function refresh() {
  if (view === "doctor") return renderDoctor();
  clear(main);
  main.appendChild(el("p", "loading", "Reading your harnesses…"));
  try {
    current = await invoke<Overview>("overview");
    renderOverview(current);
  } catch (error) {
    clear(main);
    main.appendChild(el("pre", "error", String(error)));
  }
}

// The machine changes underneath us — another window, a harness updating
// itself, our own writes. Nothing here polls, so offer an explicit re-read.
refreshButton.addEventListener("click", () => void refresh());

void refresh();
