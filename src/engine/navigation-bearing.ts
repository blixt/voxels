import { worldUnitsToMeters } from "./scale.ts";

export interface NavigationBearingInput {
  viewerPosition: readonly [number, number, number];
  viewerYawRadians: number;
  targetPosition: readonly [number, number, number];
  targetName: string;
}

export interface NavigationBearing {
  targetName: string;
  distanceMeters: number;
  distanceLabel: string;
  compassLabel: string;
  turnLabel: string;
  bearingLabel: string;
}

const COMPASS_LABELS = ["E", "SE", "S", "SW", "W", "NW", "N", "NE"] as const;

export function describeNavigationBearing(input: NavigationBearingInput): NavigationBearing {
  const deltaX = input.targetPosition[0] - input.viewerPosition[0];
  const deltaZ = input.targetPosition[2] - input.viewerPosition[2];
  const distanceMeters = worldUnitsToMeters(Math.hypot(deltaX, deltaZ));
  const targetAngle = Math.atan2(deltaZ, deltaX);
  const relativeAngle = wrapRadians(targetAngle - input.viewerYawRadians);
  const compassLabel = compassLabelForAngle(targetAngle);
  const turnLabel = turnLabelForRelativeAngle(relativeAngle);
  const distanceLabel = formatDistanceMeters(distanceMeters);
  return {
    targetName: input.targetName,
    distanceMeters,
    distanceLabel,
    compassLabel,
    turnLabel,
    bearingLabel: `${compassLabel} ${distanceLabel} ${turnLabel}`,
  };
}

function compassLabelForAngle(angle: number): string {
  const normalized = wrapRadians(angle);
  const positive = normalized < 0 ? normalized + Math.PI * 2 : normalized;
  const index = Math.round(positive / (Math.PI / 4)) % COMPASS_LABELS.length;
  return COMPASS_LABELS[index]!;
}

function turnLabelForRelativeAngle(relativeAngle: number): string {
  const degrees = relativeAngle * 180 / Math.PI;
  const abs = Math.abs(degrees);
  if (abs <= 15) {
    return "ahead";
  }
  if (abs <= 60) {
    return degrees > 0 ? "ahead right" : "ahead left";
  }
  if (abs <= 135) {
    return degrees > 0 ? "right" : "left";
  }
  return "behind";
}

function formatDistanceMeters(distanceMeters: number): string {
  if (distanceMeters < 10) {
    return `${distanceMeters.toFixed(1)} m`;
  }
  if (distanceMeters < 1000) {
    return `${Math.round(distanceMeters)} m`;
  }
  return `${(distanceMeters / 1000).toFixed(1)} km`;
}

function wrapRadians(value: number): number {
  let wrapped = value;
  while (wrapped <= -Math.PI) {
    wrapped += Math.PI * 2;
  }
  while (wrapped > Math.PI) {
    wrapped -= Math.PI * 2;
  }
  return wrapped;
}
