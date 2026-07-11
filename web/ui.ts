export interface EngineStats {
  readonly position: readonly [number, number, number];
  readonly yaw: number;
  readonly pitch: number;
  readonly grounded: boolean;
  readonly residentQuads: number;
  readonly edits: number;
  readonly nearResident: number;
  readonly tracked: number;
  readonly visible: number;
  readonly drawCalls: number;
  readonly arenaPages: number;
  readonly arenaAllocatedMiB: number;
  readonly arenaCapacityMiB: number;
  readonly queued: number;
  readonly farResident: number;
  readonly frameMilliseconds: number;
  readonly fps: number;
}

export interface GlassToggleDefinition {
  readonly code: number;
  readonly label: string;
  readonly detail: string;
  readonly enabled?: boolean;
}

export interface GlassControlDeckOptions {
  readonly mount: HTMLElement;
  readonly initiallyOpen?: boolean;
  readonly requestSnapshot: () => Promise<number[]>;
  readonly setOption: (code: number, enabled: boolean) => void;
  readonly toggles: readonly GlassToggleDefinition[];
}

export interface GlassControlDeck {
  readonly visible: boolean;
  setVisible(visible: boolean): void;
  toggle(): void;
  refresh(): Promise<void>;
  destroy(): void;
}

export interface GlassMenuItem {
  readonly label?: string;
  readonly detail?: string;
  readonly danger?: boolean;
  readonly separator?: boolean;
  readonly action?: () => void;
}

const integer = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
const decimal = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

const valueAt = (values: readonly number[], index: number): number => {
  const value = values[index];
  return Number.isFinite(value) ? (value ?? 0) : 0;
};

export function decodeEngineStats(values: readonly number[]): EngineStats {
  const frameMilliseconds = Math.max(0, valueAt(values, 17));
  return {
    position: [valueAt(values, 0), valueAt(values, 1), valueAt(values, 2)],
    yaw: valueAt(values, 3),
    pitch: valueAt(values, 4),
    grounded: valueAt(values, 5) > 0.5,
    residentQuads: valueAt(values, 6),
    edits: valueAt(values, 7),
    nearResident: valueAt(values, 8),
    tracked: valueAt(values, 9),
    visible: valueAt(values, 10),
    drawCalls: valueAt(values, 11),
    arenaPages: valueAt(values, 12),
    arenaAllocatedMiB: valueAt(values, 13),
    arenaCapacityMiB: valueAt(values, 14),
    queued: valueAt(values, 15),
    farResident: valueAt(values, 16),
    frameMilliseconds,
    fps: frameMilliseconds > 0 ? 1000 / frameMilliseconds : 0,
  };
}

export function formatEngineSnapshot(stats: EngineStats): string {
  const [x, y, z] = stats.position;
  return [
    `${decimal.format(stats.fps)} fps / ${decimal.format(stats.frameMilliseconds)} ms`,
    `${integer.format(stats.visible)} visible / ${integer.format(stats.drawCalls)} draws`,
    `${integer.format(stats.nearResident)} near + ${integer.format(stats.farResident)} far / ${integer.format(stats.tracked)} tracked`,
    `${integer.format(stats.residentQuads)} quads`,
    `${decimal.format(stats.arenaAllocatedMiB)} / ${decimal.format(stats.arenaCapacityMiB)} MiB in ${integer.format(stats.arenaPages)} pages`,
    `${integer.format(stats.queued)} queued / ${integer.format(stats.edits)} edits`,
    `position ${x.toFixed(2)}, ${y.toFixed(2)}, ${z.toFixed(2)}`,
  ].join("\n");
}

function glassSurface<T extends HTMLElement>(element: T, interactive = false): T {
  element.classList.add("glass-surface");
  if (!interactive) return element;
  element.classList.add("glass-reactive");
  const move = (event: PointerEvent): void => {
    const rect = element.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
    const y = Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height));
    element.style.setProperty("--glass-x", `${(x * 100).toFixed(1)}%`);
    element.style.setProperty("--glass-y", `${(y * 100).toFixed(1)}%`);
    element.style.setProperty("--glass-lean-x", `${((0.5 - y) * 0.7).toFixed(2)}deg`);
    element.style.setProperty("--glass-lean-y", `${((x - 0.5) * 0.7).toFixed(2)}deg`);
  };
  const leave = (): void => {
    element.style.removeProperty("--glass-x");
    element.style.removeProperty("--glass-y");
    element.style.removeProperty("--glass-lean-x");
    element.style.removeProperty("--glass-lean-y");
  };
  element.addEventListener("pointermove", move);
  element.addEventListener("pointerleave", leave);
  return element;
}

export function createGlassButton(label: string, className = ""): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = `glass-button ${className}`.trim();
  button.textContent = label;
  return glassSurface(button, true);
}

export function createGlassToggle(
  definition: GlassToggleDefinition,
  onChange: (enabled: boolean) => void,
): { readonly element: HTMLButtonElement; set(enabled: boolean): void } {
  const element = createGlassButton("", "toggle-row");
  element.setAttribute("role", "switch");
  const copy = document.createElement("span");
  copy.className = "toggle-copy";
  const label = document.createElement("span");
  label.className = "toggle-label";
  label.textContent = definition.label;
  const detail = document.createElement("span");
  detail.className = "toggle-detail";
  detail.textContent = definition.detail;
  copy.append(label, detail);
  const track = document.createElement("span");
  track.className = "toggle-track";
  track.setAttribute("aria-hidden", "true");
  const thumb = document.createElement("span");
  thumb.className = "toggle-thumb";
  track.appendChild(thumb);
  element.append(copy, track);

  let enabled = definition.enabled ?? true;
  const set = (next: boolean): void => {
    enabled = next;
    element.setAttribute("aria-checked", String(next));
    element.classList.toggle("is-on", next);
  };
  set(enabled);
  element.addEventListener("click", () => {
    set(!enabled);
    onChange(enabled);
  });
  return { element, set };
}

class GlassContextMenu {
  readonly element: HTMLDivElement;
  private readonly onOutside = (event: PointerEvent): void => {
    if (!this.element.contains(event.target as Node)) this.close();
  };
  private readonly onKey = (event: KeyboardEvent): void => {
    if (event.key === "Escape") this.close();
  };

  constructor(mount: HTMLElement, items: readonly GlassMenuItem[]) {
    this.element = glassSurface(document.createElement("div"), true);
    this.element.classList.add("glass-menu");
    this.element.setAttribute("role", "menu");
    this.element.hidden = true;
    for (const item of items) {
      if (item.separator) {
        const separator = document.createElement("div");
        separator.className = "menu-separator";
        separator.setAttribute("role", "separator");
        this.element.appendChild(separator);
        continue;
      }
      const row = document.createElement("button");
      row.type = "button";
      row.className = "menu-row";
      row.setAttribute("role", "menuitem");
      row.classList.toggle("danger", item.danger ?? false);
      const label = document.createElement("span");
      label.textContent = item.label ?? "";
      row.appendChild(label);
      if (item.detail) {
        const detail = document.createElement("small");
        detail.textContent = item.detail;
        row.appendChild(detail);
      }
      row.addEventListener("click", () => {
        this.close();
        item.action?.();
      });
      this.element.appendChild(row);
    }
    mount.appendChild(this.element);
  }

  open(x: number, y: number): void {
    this.element.hidden = false;
    this.element.style.left = `${x}px`;
    this.element.style.top = `${y}px`;
    const rect = this.element.getBoundingClientRect();
    const left = Math.min(x, window.innerWidth - rect.width - 12);
    const top = Math.min(y, window.innerHeight - rect.height - 12);
    this.element.style.left = `${Math.max(12, left)}px`;
    this.element.style.top = `${Math.max(12, top)}px`;
    requestAnimationFrame(() => this.element.classList.add("is-open"));
    window.addEventListener("pointerdown", this.onOutside, { capture: true });
    window.addEventListener("keydown", this.onKey);
    this.element.querySelector<HTMLButtonElement>("button")?.focus();
  }

  close(): void {
    this.element.classList.remove("is-open");
    this.element.hidden = true;
    window.removeEventListener("pointerdown", this.onOutside, { capture: true });
    window.removeEventListener("keydown", this.onKey);
  }

  destroy(): void {
    this.close();
    this.element.remove();
  }
}

class FrameHistory {
  readonly element: SVGSVGElement;
  private readonly line: SVGPathElement;
  private readonly samples: number[] = [];

  constructor() {
    const namespace = "http://www.w3.org/2000/svg";
    this.element = document.createElementNS(namespace, "svg");
    this.element.classList.add("frame-history");
    this.element.setAttribute("viewBox", "0 0 320 58");
    this.element.setAttribute("preserveAspectRatio", "none");
    this.element.setAttribute("aria-label", "Recent frame time history");
    const budget = document.createElementNS(namespace, "path");
    budget.classList.add("frame-budget");
    budget.setAttribute("d", "M 0 29 L 320 29");
    this.line = document.createElementNS(namespace, "path");
    this.line.classList.add("frame-line");
    this.element.append(budget, this.line);
  }

  push(milliseconds: number): void {
    this.samples.push(Math.min(33.3, Math.max(0, milliseconds)));
    if (this.samples.length > 64) this.samples.shift();
    const last = Math.max(1, this.samples.length - 1);
    const points = this.samples.map((sample, index) => {
      const x = (index / last) * 320;
      const y = 56 - (sample / 33.3) * 52;
      return `${index === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
    });
    this.line.setAttribute("d", points.join(" "));
    this.element.classList.toggle("over-budget", milliseconds > 18.5);
  }
}

function statCard(labelText: string): {
  readonly element: HTMLDivElement;
  readonly value: HTMLOutputElement;
  readonly detail: HTMLSpanElement;
} {
  const element = document.createElement("div");
  element.className = "stat-card";
  const label = document.createElement("span");
  label.className = "stat-label";
  label.textContent = labelText;
  const value = document.createElement("output");
  value.className = "stat-value";
  value.textContent = "—";
  const detail = document.createElement("span");
  detail.className = "stat-detail";
  detail.textContent = "waiting";
  element.append(label, value, detail);
  return { element, value, detail };
}

function sectionHeading(label: string, detail?: string): HTMLDivElement {
  const heading = document.createElement("div");
  heading.className = "deck-section-heading";
  const title = document.createElement("span");
  title.textContent = label;
  heading.appendChild(title);
  if (detail) {
    const note = document.createElement("small");
    note.textContent = detail;
    heading.appendChild(note);
  }
  return heading;
}

export function createGlassControlDeck(options: GlassControlDeckOptions): GlassControlDeck {
  const root = document.createElement("div");
  root.className = "engine-ui";

  const brand = glassSurface(document.createElement("div"), true);
  brand.classList.add("engine-brand");
  const brandName = document.createElement("strong");
  const brandMark = document.createElement("span");
  brandMark.textContent = "v";
  brandName.append(brandMark, document.createTextNode("oxels"));
  const scale = document.createElement("small");
  scale.textContent = "10 CM CUBES · RUST / WGPU";
  brand.append(brandName, scale);

  const launcher = createGlassButton("", "control-launcher");
  launcher.setAttribute("aria-keyshortcuts", "F3");
  const live = document.createElement("span");
  live.className = "live-dot";
  live.setAttribute("aria-hidden", "true");
  const launcherLabel = document.createElement("span");
  launcherLabel.textContent = "MISSION CONTROL";
  const launcherFps = document.createElement("output");
  launcherFps.textContent = "— FPS";
  launcher.append(live, launcherLabel, launcherFps);

  const panel = glassSurface(document.createElement("section"), true);
  panel.id = "engine-control-deck";
  panel.classList.add("control-deck");
  panel.setAttribute("aria-label", "Voxel engine mission control");
  launcher.setAttribute("aria-controls", panel.id);
  const header = document.createElement("header");
  header.className = "deck-header";
  const headingCopy = document.createElement("div");
  const eyebrow = document.createElement("span");
  eyebrow.className = "deck-eyebrow";
  eyebrow.textContent = "ENGINE LAB";
  const heading = document.createElement("h1");
  heading.textContent = "Mission control";
  headingCopy.append(eyebrow, heading);
  const headerActions = document.createElement("div");
  headerActions.className = "deck-header-actions";
  const menuButton = createGlassButton("•••", "icon-button");
  menuButton.setAttribute("aria-label", "Open mission control menu");
  menuButton.setAttribute("aria-haspopup", "menu");
  const closeButton = createGlassButton("×", "icon-button");
  closeButton.setAttribute("aria-label", "Close mission control");
  headerActions.append(menuButton, closeButton);
  header.append(headingCopy, headerActions);

  const fps = statCard("FRAME");
  const scene = statCard("SCENE");
  const geometry = statCard("GEOMETRY");
  const memory = statCard("GPU ARENA");
  const statGrid = document.createElement("div");
  statGrid.className = "stat-grid";
  statGrid.append(fps.element, scene.element, geometry.element, memory.element);
  const history = new FrameHistory();
  const chartWrap = document.createElement("div");
  chartWrap.className = "history-wrap";
  const chartLegend = document.createElement("div");
  chartLegend.className = "history-legend";
  chartLegend.append(
    document.createTextNode("FRAME HISTORY"),
    Object.assign(document.createElement("small"), { textContent: "16.7 MS BUDGET" }),
  );
  chartWrap.append(chartLegend, history.element);

  const toggles = document.createElement("div");
  toggles.className = "toggle-list";
  const toggleControls = new Map<number, ReturnType<typeof createGlassToggle>>();
  const optionState = new Map<number, boolean>();
  for (const definition of options.toggles) {
    const enabled = definition.enabled ?? true;
    optionState.set(definition.code, enabled);
    const control = createGlassToggle(definition, (next) => {
      optionState.set(definition.code, next);
      options.setOption(definition.code, next);
    });
    toggleControls.set(definition.code, control);
    toggles.appendChild(control.element);
  }

  const inspect = document.createElement("div");
  inspect.className = "inspect-strip";
  const position = document.createElement("output");
  position.textContent = "POSITION —";
  const workload = document.createElement("output");
  workload.textContent = "WORK —";
  inspect.append(position, workload);

  const footer = document.createElement("footer");
  footer.className = "deck-footer";
  const copyButton = createGlassButton("Copy snapshot", "deck-action");
  const hint = document.createElement("span");
  hint.textContent = "F3 TO TOGGLE";
  footer.append(copyButton, hint);

  panel.append(
    header,
    sectionHeading("LIVE TELEMETRY", "500 MS SAMPLE"),
    statGrid,
    chartWrap,
    sectionHeading("RENDER LAB", "LIVE RUST SWITCHES"),
    toggles,
    sectionHeading("INSPECT"),
    inspect,
    footer,
  );
  root.append(brand, launcher, panel);
  options.mount.appendChild(root);

  let latest: EngineStats | undefined;
  let visible = options.initiallyOpen ?? false;
  let busy = false;
  let destroyed = false;
  let compact = false;

  const copySnapshot = (): void => {
    if (!latest) return;
    void navigator.clipboard
      .writeText(formatEngineSnapshot(latest))
      .then(() => {
        const original = copyButton.textContent;
        copyButton.textContent = "Copied";
        window.setTimeout(() => {
          copyButton.textContent = original;
        }, 900);
      })
      .catch(() => {
        copyButton.textContent = "Clipboard unavailable";
      });
  };
  const resetOptions = (): void => {
    for (const [code, control] of toggleControls) {
      optionState.set(code, true);
      control.set(true);
      options.setOption(code, true);
    }
  };
  const menu = new GlassContextMenu(root, [
    { label: "Copy diagnostics", detail: "Clipboard", action: copySnapshot },
    {
      label: "Compact telemetry",
      detail: "Toggle density",
      action: () => {
        compact = !compact;
        panel.classList.toggle("is-compact", compact);
      },
    },
    { separator: true },
    { label: "Reset renderer features", action: resetOptions },
    { label: "Hide mission control", detail: "F3", action: () => setVisible(false) },
  ]);

  const update = (stats: EngineStats): void => {
    latest = stats;
    launcherFps.textContent = `${integer.format(stats.fps)} FPS`;
    launcher.classList.toggle("is-slow", stats.frameMilliseconds > 20);
    fps.value.textContent = `${decimal.format(stats.frameMilliseconds)} ms`;
    fps.detail.textContent = `${integer.format(stats.fps)} FPS`;
    fps.element.classList.toggle("is-warning", stats.frameMilliseconds > 20);
    scene.value.textContent = `${integer.format(stats.visible)} visible`;
    scene.detail.textContent = `${integer.format(stats.drawCalls)} draws · ${integer.format(stats.nearResident)} near + ${integer.format(stats.farResident)} far`;
    geometry.value.textContent = `${integer.format(stats.residentQuads)} quads`;
    geometry.detail.textContent = `${integer.format(stats.tracked)} tracked chunks`;
    memory.value.textContent = `${decimal.format(stats.arenaAllocatedMiB)} MiB`;
    memory.detail.textContent = `${decimal.format(stats.arenaCapacityMiB)} capacity · ${integer.format(stats.arenaPages)} pages`;
    const [x, y, z] = stats.position;
    position.textContent = `POSITION  ${x.toFixed(2)}  ${y.toFixed(2)}  ${z.toFixed(2)}`;
    workload.textContent = `WORK  ${integer.format(stats.queued)} QUEUED  ·  ${integer.format(stats.edits)} EDITS  ·  ${stats.grounded ? "GROUNDED" : "AIRBORNE"}`;
    history.push(stats.frameMilliseconds);
  };

  const refresh = async (): Promise<void> => {
    if (destroyed || !visible || busy) return;
    busy = true;
    try {
      update(decodeEngineStats(await options.requestSnapshot()));
    } finally {
      busy = false;
    }
  };
  const setVisible = (next: boolean): void => {
    visible = next;
    panel.hidden = !next;
    panel.classList.toggle("is-open", next);
    launcher.setAttribute("aria-expanded", String(next));
    launcher.classList.toggle("is-active", next);
    if (next) void refresh();
    else menu.close();
  };

  launcher.addEventListener("click", () => setVisible(!visible));
  closeButton.addEventListener("click", () => setVisible(false));
  copyButton.addEventListener("click", copySnapshot);
  menuButton.addEventListener("click", (event) => {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu.open(rect.right - 220, rect.bottom + 8);
  });
  panel.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    menu.open(event.clientX, event.clientY);
  });
  setVisible(visible);
  const timer = window.setInterval(() => void refresh(), 500);

  return {
    get visible() {
      return visible;
    },
    setVisible,
    toggle: () => setVisible(!visible),
    refresh,
    destroy: () => {
      destroyed = true;
      window.clearInterval(timer);
      menu.destroy();
      root.remove();
    },
  };
}
