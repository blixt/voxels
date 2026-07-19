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

function isVisuallyValid(report) {
  return (
    report.pose?.errorMetres <= 0.025 &&
    report.image?.relativeMeanLumaDelta <= 0.04 &&
    report.image?.meanAbsoluteLinearLumaDelta <= 0.025 &&
    report.image?.catastrophicDarkFraction <= 0.01 &&
    report.image?.nearBlackPixelFraction?.before <= 0.1 &&
    report.image?.nearBlackPixelFraction?.after <= 0.1 &&
    report.image?.isolatedSkyExposurePixels?.before?.count === 0 &&
    report.image?.isolatedSkyExposurePixels?.after?.count === 0 &&
    report.image?.ssim >= 0.97
  );
}

function annotateFrame(source, destination, label, accent, report) {
  const poseErrorMillimetres = (report.pose.errorMetres * 1_000).toFixed(1);
  const catastrophicPercent = (report.image.catastrophicDarkFraction * 100).toFixed(1);
  const font = "/System/Library/Fonts/Supplemental/Arial.ttf";
  execFileSync(
    "magick",
    [
      source,
      "-resize",
      "1920x1080^",
      "-gravity",
      "center",
      "-extent",
      "1920x1080",
      "-gravity",
      "northwest",
      "(",
      source,
      "-crop",
      "564x217+25+201",
      "+repage",
      "-filter",
      "point",
      "-resize",
      "720x277",
      "-bordercolor",
      accent,
      "-border",
      "6",
      ")",
      "-geometry",
      "+1160+665",
      "-composite",
      "-fill",
      "#070b12c8",
      "-draw",
      "rectangle 0,0 1920,145",
      "-draw",
      "rectangle 0,960 1920,1080",
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
      "#dce5f5",
      "-pointsize",
      "26",
      "-annotate",
      "+40+1005",
      `${catastrophicPercent}% of sampled regions changed luminance by at least 2x`,
      "-annotate",
      "+40+1045",
      `SSIM ${report.image.ssim.toFixed(3)} · isolated sky pixels 0 · LOD transition quads ${report.lod.transitionQuadsBefore} → ${report.lod.transitionQuadsAfter}`,
      "-fill",
      "#f2f6ff",
      "-pointsize",
      "22",
      "-annotate",
      "+1170+650",
      "MEASURED VALLEY · PIXEL-NEAREST ZOOM",
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
  let captureDirectory;
  let report;
  let lastExitStatus;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    captureDirectory = path.join(temporaryDirectory, `attempt-${attempt}`);
    await mkdir(captureDirectory);
    const run = spawnSync(process.execPath, [harness], {
      cwd: process.cwd(),
      env: {
        ...process.env,
        VOXELS_LOD_TEST_OUTPUT: captureDirectory,
        VOXELS_LOD_TEST_RECORD_VIDEO: "1",
        VOXELS_LOD_TEST_SSAO: "0",
      },
      encoding: "utf8",
    });
    lastExitStatus = run.status;
    if (run.stdout) process.stdout.write(run.stdout);
    if (run.stderr) process.stderr.write(run.stderr);
    try {
      const candidate = JSON.parse(
        await readFile(path.join(captureDirectory, "report.json"), "utf8"),
      );
      if (isVisuallyValid(candidate)) {
        report = candidate;
        break;
      }
      console.warn(`LOD video acquisition attempt ${attempt} failed visual validation; retrying`);
    } catch {
      console.warn(`LOD video acquisition attempt ${attempt} did not settle; retrying`);
    }
  }
  if (!report) {
    throw new Error(
      `LOD capture did not produce a visually valid report after three attempts (last exit ${lastExitStatus ?? "unknown"})`,
    );
  }
  if (report.mode !== "transition" || !report.pose?.before || !report.pose?.after) {
    throw new Error("LOD capture report does not contain a completed transition comparison");
  }

  const reportPath = path.join(captureDirectory, "report.json");
  const before = path.join(captureDirectory, "before.png");
  const after = path.join(captureDirectory, "after.png");
  const rawWebm = path.join(captureDirectory, "transition-raw.webm");
  const annotatedBefore = path.join(temporaryDirectory, "annotated-before.png");
  const annotatedAfter = path.join(temporaryDirectory, "annotated-after.png");
  annotateFrame(before, annotatedBefore, "LOD CENTER A · BEFORE", "#5ce1e6", report);
  annotateFrame(after, annotatedAfter, "LOD CENTER B · AFTER", "#ffb454", report);

  const comparisonMp4 = path.join(DELIVERY_DIRECTORY, "voxels-lod-transition-comparison.mp4");
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
  const rawStartSeconds = Math.max(0, (report.videoMarkers?.beforeSeconds ?? 1) - 1);
  const rawDurationSeconds = Math.max(
    3,
    (report.videoMarkers?.afterSeconds ?? rawStartSeconds + 8) - rawStartSeconds + 2.5,
  );
  execFileSync(
    "ffmpeg",
    [
      "-y",
      "-hide_banner",
      "-loglevel",
      "warning",
      "-ss",
      String(rawStartSeconds),
      "-i",
      rawWebm,
      "-t",
      String(rawDurationSeconds),
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
    copyFile(comparisonMp4, path.join(ARTIFACT_DIRECTORY, "transition-comparison.mp4")),
    copyFile(rawMp4, path.join(ARTIFACT_DIRECTORY, "transition-raw.mp4")),
  ]);

  console.log(
    JSON.stringify(
      {
        ok: true,
        harnessViolations: report.violations,
        comparisonMp4,
        rawMp4,
        artifacts: ARTIFACT_DIRECTORY,
        metrics: {
          poseErrorMillimetres: report.pose.errorMetres * 1_000,
          catastrophicDarkFraction: report.image.catastrophicDarkFraction,
          isolatedSkyExposurePixels: report.image.isolatedSkyExposurePixels,
          nearBlackPixelFraction: report.image.nearBlackPixelFraction,
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
