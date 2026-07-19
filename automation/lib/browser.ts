import { createServer as createNetServer } from "node:net";
import path from "node:path";
import {
  chromium,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type LaunchOptions,
  type Page,
  type ViewportSize,
} from "playwright";
import type { ScenarioContext } from "./scenario.ts";
import { EngineClient } from "./engine.ts";

export interface BrowserFailure {
  readonly source: "console" | "page";
  readonly page: string;
  readonly message: string;
}

export interface OpenPageOptions {
  readonly url: string;
  readonly label?: string;
  readonly viewport?: ViewportSize;
  readonly deviceScaleFactor?: number;
  readonly storageState?: BrowserContextOptions["storageState"];
  readonly recordVideo?: boolean;
  readonly videoFilename?: string;
  readonly engine?: boolean;
  readonly beforeNavigate?: (context: BrowserContext, page: Page) => void | Promise<void>;
}

export interface ScreenshotOptions {
  readonly filename?: string;
  readonly fullPage?: boolean;
}

export class BrowserViewport {
  readonly context: BrowserContext;
  readonly failures: BrowserFailure[];
  readonly page: Page;
  readonly engine: EngineClient;
  readonly label: string;

  readonly #scenario: ScenarioContext;

  constructor(
    scenario: ScenarioContext,
    context: BrowserContext,
    page: Page,
    label: string,
    failures: BrowserFailure[],
  ) {
    this.#scenario = scenario;
    this.context = context;
    this.failures = failures;
    this.page = page;
    this.label = label;
    this.engine = new EngineClient(page);
  }

  async screenshot(label: string, options: ScreenshotOptions = {}): Promise<string> {
    const filename = options.filename ?? `${label}.png`;
    const destination = this.#scenario.artifacts.resolve(filename);
    await this.page.screenshot({ path: destination, fullPage: options.fullPage ?? false });
    return this.#scenario.artifacts.record(label, destination, "image/png");
  }

  async close(): Promise<void> {
    await this.context.close();
  }
}

export class BrowserCapability {
  readonly failures: BrowserFailure[] = [];

  readonly #scenario: ScenarioContext;
  readonly #browser: Browser;
  readonly #contexts: BrowserContext[] = [];
  readonly #videos: {
    readonly page: Page;
    readonly label: string;
    readonly filename: string;
  }[] = [];
  readonly #warningPattern: RegExp;
  #closePromise: Promise<void> | undefined;

  private constructor(scenario: ScenarioContext, browser: Browser, warningPattern: RegExp) {
    this.#scenario = scenario;
    this.#browser = browser;
    this.#warningPattern = warningPattern;
  }

  get version(): string {
    return this.#browser.version();
  }

  static async start(
    scenario: ScenarioContext,
    options: { readonly warningPattern?: RegExp; readonly launch?: LaunchOptions } = {},
  ): Promise<BrowserCapability> {
    if (!scenario.definition.uses.browser && scenario.definition.uses.viewport !== "browser") {
      throw new Error(`scenario ${scenario.definition.id} did not declare browser automation`);
    }
    const browser = await chromium.launch(options.launch ?? chromeWebGpuLaunchOptions());
    const capability = new BrowserCapability(
      scenario,
      browser,
      options.warningPattern ?? /wgpu|webgpu|panic|unreachable|runtimeerror/iu,
    );
    scenario.defer("browser", () => capability.close());
    return capability;
  }

  async open(options: OpenPageOptions): Promise<BrowserViewport> {
    const label = options.label ?? `page-${this.#contexts.length + 1}`;
    const viewport = options.viewport ?? { width: 1280, height: 720 };
    const contextOptions: BrowserContextOptions = {
      viewport,
      deviceScaleFactor: options.deviceScaleFactor ?? 1,
      ...(options.storageState === undefined ? {} : { storageState: options.storageState }),
    };
    if (options.recordVideo) {
      contextOptions.recordVideo = {
        dir: await this.#scenario.artifacts.directoryFor("video"),
        size: viewport,
      };
    }
    const context = await this.#browser.newContext(contextOptions);
    this.#contexts.push(context);
    const page = await context.newPage();
    const failures: BrowserFailure[] = [];
    const recordFailure = (failure: BrowserFailure): void => {
      failures.push(failure);
      this.failures.push(failure);
    };
    page.on("pageerror", (error) => {
      recordFailure({ source: "page", page: label, message: error.message });
    });
    page.on("console", (message) => {
      if (isBrowserConsoleFailure(message.type(), message.text(), this.#warningPattern)) {
        recordFailure({ source: "console", page: label, message: message.text() });
      }
    });
    await options.beforeNavigate?.(context, page);
    await page.goto(options.url, { waitUntil: "domcontentloaded" });
    const browserViewport = new BrowserViewport(this.#scenario, context, page, label, failures);
    if (options.recordVideo) {
      this.#videos.push({
        page,
        label,
        filename: options.videoFilename ?? videoArtifactName(label),
      });
    }
    if (options.engine ?? true) await browserViewport.engine.ready();
    return browserViewport;
  }

  close(): Promise<void> {
    this.#closePromise ??= this.#close();
    return this.#closePromise;
  }

  async #close(): Promise<void> {
    const errors: unknown[] = [];
    for (const context of this.#contexts.splice(0).toReversed()) {
      try {
        await context.close();
      } catch (error) {
        errors.push(error);
      }
    }
    for (const video of this.#videos.splice(0)) {
      try {
        const recording = video.page.video();
        if (recording === null) throw new Error(`page ${video.label} has no video recording`);
        const destination = this.#scenario.artifacts.resolve(video.filename);
        await recording.saveAs(destination);
        this.#scenario.artifacts.record(`${video.label} video`, destination, "video/webm");
      } catch (error) {
        errors.push(error);
      }
    }
    try {
      await this.#browser.close();
    } catch (error) {
      errors.push(error);
    }
    if (errors.length > 0) throw new AggregateError(errors, "browser cleanup failed");
  }

  assertHealthy(): void {
    if (this.failures.length === 0) return;
    throw new Error(
      this.failures
        .map((failure) => `${failure.source} (${failure.page}): ${failure.message}`)
        .join("\n"),
    );
  }
}

export function chromeWebGpuLaunchOptions(): LaunchOptions {
  return {
    channel: "chrome",
    headless: false,
    args: [
      "--headless=new",
      "--disable-features=LocalNetworkAccessChecks",
      "--no-sandbox",
      "--hide-scrollbars",
    ],
  };
}

export async function reserveEphemeralPort(): Promise<number> {
  const probe = createNetServer();
  await new Promise<void>((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const address = probe.address();
  if (!address || typeof address === "string") throw new Error("could not reserve a TCP port");
  await new Promise<void>((resolve, reject) =>
    probe.close((error) => (error ? reject(error) : resolve())),
  );
  return address.port;
}

export function videoArtifactName(pageLabel: string): string {
  return `${path.basename(pageLabel).replaceAll(/[^a-zA-Z0-9._-]+/gu, "-")}.webm`;
}

export function isBrowserConsoleFailure(
  type: string,
  text: string,
  warningPattern: RegExp,
): boolean {
  return type === "error" || (type === "warning" && warningPattern.test(text));
}
