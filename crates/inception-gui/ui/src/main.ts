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
};
type Overview = { scope: string; providers: Provider[]; skills: Row[]; servers: Row[] };

type Copy = {
  path: string; providers: string[]; description: string; body: string;
  hash: string; deprecated: boolean; readable: boolean;
};
type Comparison = { left: string; right: string; lines: DiffLine[] };
type Detail = {
  name: string;
  status: string;
  copies: Copy[];
  comparison: Comparison | null;
};

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

  const label = el("span", row.status === "problem" ? "rowname dim" : "rowname", row.name);
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

  const open = (name: string) => void openDetail(name);

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
  matrix.appendChild(legend);

  main.appendChild(matrix);
}

// ---------------------------------------------------------------------------
// Detail drawer
// ---------------------------------------------------------------------------

function closeDrawer() {
  drawer.hidden = true;
  scrim.hidden = true;
  clear(drawer);
}

scrim.addEventListener("click", closeDrawer);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !drawer.hidden) closeDrawer();
});

function drawerShell(name: string, status: string): HTMLElement {
  clear(drawer);
  drawer.hidden = false;
  scrim.hidden = false;

  const head = el("div", "drawer-head");
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

/** Highlight the span where two descriptions diverge, rather than the whole line. */
function diffSpans(a: string, b: string): [string, string, string] {
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head += 1;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail += 1;
  }
  return [a.slice(0, head), a.slice(head, a.length - tail), a.slice(a.length - tail)];
}

function renderPaper(copy: Copy, other: Copy | undefined): HTMLElement {
  const wrap = el("div", "paperwrap");

  const head = el("div", "paperhead");
  head.appendChild(el("span", undefined, copy.path));
  if (copy.deprecated) head.appendChild(el("span", "flag", "deprecated location"));
  if (!copy.readable) head.appendChild(el("span", "flag", "unreadable"));
  wrap.appendChild(head);

  const paper = el("div", "paper");
  paper.appendChild(el("span", "key", "description: "));

  if (other && other.description && other.description !== copy.description) {
    const [before, middle, after] = diffSpans(copy.description, other.description);
    paper.appendChild(document.createTextNode(before));
    paper.appendChild(el("span", "differ", middle));
    paper.appendChild(document.createTextNode(after));
  } else {
    paper.appendChild(document.createTextNode(copy.description || "(none)"));
  }

  paper.appendChild(document.createTextNode("\n\n" + copy.body));
  wrap.appendChild(paper);
  wrap.appendChild(el("div", "hash", copy.hash));
  return wrap;
}

async function openDetail(name: string) {
  const body = drawerShell(name, "reading…");
  try {
    const detail = await invoke<Detail>("detail", { name });
    clear(body);
    (drawer.querySelector(".status") as HTMLElement).textContent =
      detail.copies.length === 1
        ? "1 copy"
        : `${detail.copies.length} copies · ${detail.status}`;
    (drawer.querySelector(".status") as HTMLElement).className =
      `status${detail.status === "drifted" ? " drifted" : ""}`;

    const papers = el("div", `papers${detail.copies.length === 1 ? " one" : ""}`);
    detail.copies.forEach((copy, index) => {
      const other = detail.copies.length === 2 ? detail.copies[1 - index] : undefined;
      papers.appendChild(renderPaper(copy, other));
    });
    body.appendChild(papers);

    if (detail.comparison) {
      body.appendChild(el("div", "sectiontitle", "What differs"));
      const head = el("div", "paperhead");
      head.appendChild(el("span", undefined, `- ${detail.comparison.left}`));
      body.appendChild(head);
      const head2 = el("div", "paperhead");
      head2.appendChild(el("span", undefined, `+ ${detail.comparison.right}`));
      body.appendChild(head2);
      body.appendChild(renderDiff(detail.comparison.lines));
    }

    const actions = el("div", "actions");
    actions.appendChild(
      el("span", "count", "Copies this skill to every harness that doesn't have it."),
    );
    const deploy = el("button", "btn primary", "Plan a deploy");
    deploy.type = "button";
    deploy.addEventListener("click", () => void openPlan(name));
    actions.appendChild(deploy);
    body.appendChild(actions);
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
  }
}

// ---------------------------------------------------------------------------
// Deploy
// ---------------------------------------------------------------------------

function renderDiff(lines: DiffLine[]): HTMLElement {
  const diff = el("div", "diff");
  for (const line of lines) {
    const sign = line.kind === "add" ? "+ " : line.kind === "del" ? "- " : "  ";
    diff.appendChild(el("div", line.kind, sign + line.text));
  }
  return diff;
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

  if (step.diff.length) body.appendChild(renderDiff(step.diff));

  wrap.appendChild(body);
  return wrap;
}

async function openPlan(name: string) {
  const targets = (current?.providers ?? []).map((p) => p.id);
  const body = drawerShell(`deploy ${name}`, "planning…");

  try {
    const plan = await invoke<PlanView>("plan_deploy", { name, targets });
    clear(body);
    const status = drawer.querySelector(".status") as HTMLElement;
    status.className = "status";
    status.textContent =
      plan.steps.length === 0
        ? "nothing to do"
        : `${plan.steps.length} write${plan.steps.length === 1 ? "" : "s"} · ${plan.served.length} served`;

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
    const cancel = el("button", "btn", "Cancel");
    cancel.type = "button";
    cancel.addEventListener("click", () => void openDetail(name));
    const go = el("button", "btn primary",
      `Write ${plan.steps.length} file${plan.steps.length === 1 ? "" : "s"}`);
    go.type = "button";
    go.addEventListener("click", () => {
      go.disabled = true;
      cancel.disabled = true;
      go.textContent = "Writing…";
      void runPlan(name, targets);
    });
    actions.appendChild(cancel);
    actions.appendChild(go);
    body.appendChild(actions);
  } catch (error) {
    clear(body);
    body.appendChild(el("pre", "error", String(error)));
  }
}

async function runPlan(name: string, targets: string[]) {
  try {
    const outcomes = await invoke<Outcome[]>("apply", { name, targets });
    const body = drawerShell(`deploy ${name}`, "done");
    const failed = outcomes.filter((o) => !o.ok).length;
    (drawer.querySelector(".status") as HTMLElement).textContent =
      failed === 0 ? `${outcomes.length} written` : `${failed} failed`;

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
    const body = drawerShell(`deploy ${name}`, "failed");
    body.appendChild(el("pre", "error", String(error)));
  }
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

type Finding = {
  severity: "security" | "warning" | "info";
  provider: string | null;
  message: string;
  suggestion: string | null;
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

void refresh();
