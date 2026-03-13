#!/usr/bin/env bash
set -euo pipefail

# Quick pre-check: typecheck
bun run typecheck 2>&1 >/dev/null

# Run tests (must pass — correctness gate)
bun test 2>&1 | tail -5

# Run the game-stream profile benchmark (crossing-d2 scenario)
# Uses 5 iterations after 2 warmup for stable numbers
OUTPUT=$(bun run scripts/profile-game-stream.ts \
  --iterations=5 \
  --warmup=2 \
  --seed=1337 \
  --radius=5 \
  --generate-budget=6 \
  --mesh-budget=4 \
  --far-band-budget=1 \
  --chunk-delta=2 2>&1)

# Extract crossing-d2 scenario (first JSON line)
CROSSING_D2=$(echo "$OUTPUT" | grep '"crossing-d2"' | head -1)

if [ -z "$CROSSING_D2" ]; then
  echo "ERROR: No crossing-d2 output found"
  echo "$OUTPUT"
  exit 1
fi

# Extract metrics using bun for reliable JSON parsing
bun -e "
const data = $CROSSING_D2;
const totalMs = data.totalStreamMs.avg + data.totalMeshMs.avg + data.totalFarFieldMs.avg;
console.log('METRIC total_ms=' + totalMs.toFixed(2));
console.log('METRIC stream_ms=' + data.totalStreamMs.avg.toFixed(2));
console.log('METRIC mesh_ms=' + data.totalMeshMs.avg.toFixed(2));
console.log('METRIC far_field_ms=' + data.totalFarFieldMs.avg.toFixed(2));
console.log('METRIC chunk_gen_ms=' + data.totalChunkGenerationMs.avg.toFixed(2));
console.log('METRIC max_frame_ms=' + data.maxFrameWorkMs.avg.toFixed(2));
console.log('METRIC far_sample_cache_ms=' + data.totalFarFieldSampleCacheMs.avg.toFixed(2));
console.log('METRIC far_mesh_build_ms=' + data.totalFarFieldMeshBuildMs.avg.toFixed(2));
"
