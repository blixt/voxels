import packageJson from "../package.json" assert { type: "json" };

const miseConfig = await Bun.file("mise.toml").text();
const current = {
  chrome: await chromeVersion(),
  bun: readMiseVersion(miseConfig, "bun"),
  mise: await commandVersion("mise --version"),
  bunTypes: versionString(packageJson.devDependencies["@types/bun"]),
  typescript: versionString(packageJson.devDependencies.typescript),
  webgpuTypes: versionString(packageJson.devDependencies["@webgpu/types"]),
};

const latest = {
  chrome: await latestChromeStable(),
  bun: await latestGithubRelease("oven-sh", "bun", /^bun-v/),
  mise: await latestGithubRelease("jdx", "mise", /^v/),
  bunTypes: await latestNpmVersion("@types/bun"),
  typescript: await latestNpmVersion("typescript"),
  webgpuTypes: await latestNpmVersion("@webgpu/types"),
};

const projectStatuses = [
  report("Chrome stable", current.chrome, latest.chrome),
  report("Bun pin", current.bun, latest.bun),
  report("@types/bun", current.bunTypes, latest.bunTypes),
  report("TypeScript", current.typescript, latest.typescript),
  report("@webgpu/types", current.webgpuTypes, latest.webgpuTypes),
];
const toolStatuses = [
  report("mise binary", current.mise, latest.mise),
];

console.log("Project-managed versions");
for (const status of projectStatuses) {
  console.log(formatStatus(status));
}
console.log("");
console.log("Global tooling");
for (const status of toolStatuses) {
  console.log(formatStatus(status));
}
console.log("");
console.log("Sources");
console.log("- Chrome stable mac: https://versionhistory.googleapis.com/v1/chrome/platforms/mac/channels/stable/versions?pageSize=1");
console.log("- Bun latest release: https://github.com/oven-sh/bun/releases/latest");
console.log("- mise latest release: https://github.com/jdx/mise/releases/latest");
console.log("- TypeScript latest: https://registry.npmjs.org/typescript/latest");
console.log("- @types/bun latest: https://registry.npmjs.org/@types/bun/latest");
console.log("- @webgpu/types latest: https://registry.npmjs.org/@webgpu/types/latest");

if (projectStatuses.some((status) => !status.currentIsLatest)) {
  process.exitCode = 1;
}

interface StatusRow {
  label: string;
  current: string;
  latest: string;
  currentIsLatest: boolean;
}

function report(label: string, currentVersion: string, latestVersion: string): StatusRow {
  return {
    label,
    current: currentVersion,
    latest: latestVersion,
    currentIsLatest: normalize(currentVersion) === normalize(latestVersion),
  };
}

function formatStatus(status: StatusRow): string {
  const indicator = status.currentIsLatest ? "up-to-date" : "update-needed";
  return `- ${status.label}: ${status.current} (latest ${status.latest}) [${indicator}]`;
}

function versionString(value: string | undefined): string {
  return value?.replace(/^[^\d]*/, "") ?? "unknown";
}

function readMiseVersion(source: string, toolName: string): string {
  const match = source.match(new RegExp(`^${toolName}\\s*=\\s*\"([^\"]+)\"$`, "m"));
  if (!match) {
    throw new Error(`Unable to find ${toolName} in mise.toml`);
  }
  return match[1]!;
}

async function chromeVersion(): Promise<string> {
  const proc = Bun.spawn(["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "--version"], {
    stdout: "pipe",
    stderr: "ignore",
  });
  const text = await new Response(proc.stdout).text();
  return text.trim().replace(/^Google Chrome\s+/, "");
}

async function commandVersion(command: string): Promise<string> {
  const proc = Bun.spawn(["zsh", "-lc", command], { stdout: "pipe", stderr: "ignore" });
  const text = await new Response(proc.stdout).text();
  return text.trim().split(/\s+/).at(0) ?? "unknown";
}

async function latestChromeStable(): Promise<string> {
  const response = await fetch("https://versionhistory.googleapis.com/v1/chrome/platforms/mac/channels/stable/versions?pageSize=1");
  if (!response.ok) {
    throw new Error(`Chrome version history request failed: ${response.status}`);
  }
  const payload = await response.json() as { versions?: Array<{ version?: string }> };
  return payload.versions?.[0]?.version ?? "unknown";
}

async function latestGithubRelease(owner: string, repo: string, prefix: RegExp): Promise<string> {
  const response = await fetch(`https://api.github.com/repos/${owner}/${repo}/releases/latest`, {
    headers: { "user-agent": "voxels-version-check" },
  });
  if (!response.ok) {
    throw new Error(`GitHub release request failed for ${owner}/${repo}: ${response.status}`);
  }
  const payload = await response.json() as { tag_name?: string };
  return normalize(payload.tag_name?.replace(prefix, "") ?? "unknown");
}

async function latestNpmVersion(pkg: string): Promise<string> {
  const response = await fetch(`https://registry.npmjs.org/${pkg}/latest`);
  if (!response.ok) {
    throw new Error(`npm registry request failed for ${pkg}: ${response.status}`);
  }
  const payload = await response.json() as { version?: string };
  return payload.version ?? "unknown";
}

function normalize(value: string): string {
  return value.replace(/^v/, "").replace(/^bun-v/, "");
}
