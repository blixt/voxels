import { expect, test } from "bun:test";

import { describeNavigationBearing } from "../src/engine/navigation-bearing.ts";
import { metersToWorldUnits } from "../src/engine/scale.ts";

test("navigation bearing reports compass distance and relative turn", () => {
  const bearing = describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [metersToWorldUnits(12), 0, 0],
    targetName: "Old Road Causeway",
  });

  expect(bearing).toMatchObject({
    targetName: "Old Road Causeway",
    distanceLabel: "12 m",
    compassLabel: "E",
    turnLabel: "ahead",
    bearingLabel: "E 12 m ahead",
  });
});

test("navigation bearing gives readable left right and behind hints", () => {
  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [0, 0, metersToWorldUnits(5)],
    targetName: "Kwama trail",
  }).turnLabel).toBe("right");

  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [-metersToWorldUnits(5), 0, 0],
    targetName: "Return path",
  }).turnLabel).toBe("behind");

  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: Math.PI / 2,
    targetPosition: [metersToWorldUnits(5), 0, 0],
    targetName: "Ash cache",
  }).turnLabel).toBe("left");
});

test("navigation bearing wraps around the negative and positive yaw boundary", () => {
  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: Math.PI - 0.02,
    targetPosition: [-metersToWorldUnits(8), 0, -metersToWorldUnits(0.1)],
    targetName: "Return path",
  }).turnLabel).toBe("ahead");

  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: -Math.PI + 0.02,
    targetPosition: [-metersToWorldUnits(8), 0, metersToWorldUnits(0.1)],
    targetName: "Return path",
  }).turnLabel).toBe("ahead");
});

test("navigation bearing formats close and far distances compactly", () => {
  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [metersToWorldUnits(4.25), 0, 0],
    targetName: "Berry bush",
  }).distanceLabel).toBe("4.3 m");

  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [metersToWorldUnits(304), 0, 0],
    targetName: "Ashland ridge",
  }).distanceLabel).toBe("304 m");

  expect(describeNavigationBearing({
    viewerPosition: [0, 0, 0],
    viewerYawRadians: 0,
    targetPosition: [metersToWorldUnits(1250), 0, 0],
    targetName: "Glass ridge",
  }).distanceLabel).toBe("1.3 km");
});
