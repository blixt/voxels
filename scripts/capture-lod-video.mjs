import { execFileSync, spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import path from "node:path";

const ARTIFACT_DIRECTORY = path.resolve(
  process.env.VOXELS_LOD_VIDEO_ARTIFACTS ?? "target/lod-video",
);
const DELIVERY_DIRECTORY = path.resolve(
  process.env.VOXELS_LOD_VIDEO_DELIVERY ?? path.join(homedir(), "Desktop"),
);
const temporaryDirectory = await mkdtemp(path.join(tmpdir(), "voxels-lod-video-"));
const harness = path.resolve("scripts/browser-lod-transition.mjs");

function requireTool(tool) {
  try {
    execFileSync(tool, ["-version"], { stdio: "ignore" });
  } catch {
    throw new Error(`${tool} is required to capture the LOD comparison video`);
  }
}

function annotateFrame(source, destination, label, accent, report) {
  const poseErrorMillimetres = (report.pose.errorMetres * 1_000).toFixed(1);
  const catastrophicPercent = (report.image.catastrophicDarkFraction * 100).toFixed(1);
  const font = "/System/Library/Fonts/Supplemental/Arial.ttf";
  execFileSync(
    "magick",
    [
      "-size",
      "1920x1080",
      "xc:#090d14",
      "(",
      source,
      "-resize",
      "920x518",
      ")",
      "-geometry",
      "+20+281",
      "-composite",
      "(",
      source,
      "-crop",
      "564x217+25+201",
      "+repage",
      "-filter",
      "point",
      "-resize",
      "920x354",
      ")",
      "-geometry",
      "+980+363",
      "-composite",
      "-font",
      font,
      "-fill",
      accent,
      "-pointsize",
      "48",
      "-annotate",
      "+40+75",
      label,
      "-fill",
      "#f2f6ff",
      "-pointsize",
      "28",
      "-annotate",
      "+40+130",
      `Same camera pose · ${poseErrorMillimetres} mm return error`,
      "-fill",
      "#9cabc2",
      "-pointsize",
      "24",
      "-annotate",
      "+40+245",
      "FULL FRAME",
      "-annotate",
      "+1000+325",
      "MEASURED VALLEY REGION · PIXEL-NEAREST ZOOM",
      "-fill",
      "#dce5f5",
      "-pointsize",
      "26",
      "-annotate",
      "+40+930",
      `${catastrophicPercent}% of sampled regions changed luminance by at least 2x`,
      "-annotate",
      "+40+970",
      `SSIM ${report.image.ssim.toFixed(3)} · LOD transition quads ${report.lod.transitionQuadsBefore} → ${report.lod.transitionQuadsAfter}`,
      "-fill",
      "#728198",
      "-pointsize",
      "22",
      "-annotate",
      "+40+1020",
      "The video alternates only the LOD ownership center; world time, weather, and camera are fixed.",
      destination,
    ],
    { stdio: "inherit" },
  );
}

requireTool("ffmpeg");
requireTool("magick");
await Promise.all([
  mkdir(ARTIFACT_DIRECTORY, { recursive: true }),
  mkdir(DELIVERY_DIRECTORY, { recursive: true }),
]);

try {
  const run = spawnSync(process.execPath, [harness], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      VOXELS_LOD_TEST_OUTPUT: temporaryDirectory,
      VOXELS_LOD_TEST_RECORD_VIDEO: "1",
    },
    encoding: "utf8",
  });
  if (run.stdout) process.stdout.write(run.stdout);
  if (run.stderr) process.stderr.write(run.stderr);

  const reportPath = path.join(temporaryDirectory, "report.json");
  let report;
  try {
    report = JSON.parse(await readFile(reportPath, "utf8"));
  } catch {
    throw new Error(
      `LOD capture did not produce a report (harness exit ${run.status ?? "unknown"})`,
    );
  }
  if (report.mode !== "transition" || !report.pose?.before || !report.pose?.after) {
    throw new Error("LOD capture report does not contain a completed transition comparison");
  }

  const before = path.join(temporaryDirectory, "before.png");
  const after = path.join(temporaryDirectory, "after.png");
  const rawWebm = path.join(temporaryDirectory, "transition-raw.webm");
  const annotatedBefore = path.join(temporaryDirectory, "annotated-before.png");
  const annotatedAfter = path.join(temporaryDirectory, "annotated-after.png");
  annotateFrame(before, annotatedBefore, "LOD CENTER A · BEFORE", "#5ce1e6", report);
  annotateFrame(after, annotatedAfter, "LOD CENTER B · AFTER", "#ffb454", report);

  const comparisonMp4 = path.join(DELIVERY_DIRECTORY, "voxels-lod-appearance-delta.mp4");
  execFileSync(
    "ffmpeg",
    [
      "-y",
      "-hide_banner",
      "-loglevel",
      "warning",
      "-loop",
      "1",
      "-framerate",
      "60",
      "-i",
      annotatedBefore,
      "-loop",
      "1",
      "-framerate",
      "60",
      "-i",
      annotatedAfter,
      "-filter_complex",
      "[0:v][1:v]blend=all_expr='if(lt(mod(T,2),1),A,B)':shortest=1,format=yuv420p[v]",
      "-map",
      "[v]",
      "-t",
      "10",
      "-r",
      "60",
      "-c:v",
      "libx264",
      "-preset",
      "medium",
      "-crf",
      "17",
      "-movflags",
      "+faststart",
      comparisonMp4,
    ],
    { stdio: "inherit" },
  );

  const rawMp4 = path.join(DELIVERY_DIRECTORY, "voxels-lod-transition-raw.mp4");
  execFileSync(
    "ffmpeg",
    [
      "-y",
      "-hide_banner",
      "-loglevel",
      "warning",
      "-i",
      rawWebm,
      "-an",
      "-c:v",
      "libx264",
      "-preset",
      "medium",
      "-crf",
      "18",
      "-pix_fmt",
      "yuv420p",
      "-movflags",
      "+faststart",
      rawMp4,
    ],
    { stdio: "inherit" },
  );

  await Promise.all([
    copyFile(before, path.join(ARTIFACT_DIRECTORY, "before.png")),
    copyFile(after, path.join(ARTIFACT_DIRECTORY, "after.png")),
    copyFile(reportPath, path.join(ARTIFACT_DIRECTORY, "report.json")),
    copyFile(rawWebm, path.join(ARTIFACT_DIRECTORY, "transition-raw.webm")),
    copyFile(comparisonMp4, path.join(ARTIFACT_DIRECTORY, "appearance-delta.mp4")),
    copyFile(rawMp4, path.join(ARTIFACT_DIRECTORY, "transition-raw.mp4")),
  ]);

  console.log(
    JSON.stringify(
      {
        ok: true,
        harnessExpectedVisualFailure: !report.ok,
        comparisonMp4,
        rawMp4,
        artifacts: ARTIFACT_DIRECTORY,
        metrics: {
          poseErrorMillimetres: report.pose.errorMetres * 1_000,
          catastrophicDarkFraction: report.image.catastrophicDarkFraction,
          ssim: report.image.ssim,
        },
      },
      null,
      2,
    ),
  );
} finally {
  await rm(temporaryDirectory, { recursive: true, force: true });
}
