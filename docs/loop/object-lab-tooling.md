# Object Lab Tooling

Use `scripts/object-lab.ts` when a world object needs isolated visual review without loading the full renderer or changing worldgen code.

Example:

```sh
mise exec -- bun run scripts/object-lab.ts --id velothi_shrine --seed 1337 --label=velothi-shrine-check
```

The lab scans for a representative landmark root unless `--world-x` and `--world-z` are provided. Each run writes into `artifacts/object-lab/<timestamp>-<label>/`:

- `contact-sheet.svg`: browser-viewable top, front, and side projections plus fit metadata, warnings, and a material legend.
- `summary.md`: run metadata, silhouette coverage, normalized dimensions, aspect ratios, sample fit diagnostics, warnings, and material counts.
- `report.json`: machine-readable root, sample bounds, material counts, sample fit, projection diagnostics, and warnings for regression tests.
- `top.ppm`, `front.ppm`, `side.ppm`: raw projection images for simple pixel inspection.

When judging silhouette and material quality, start with `contact-sheet.svg`, then check `summary.md` for:

- very low coverage, which usually means the object is too sparse for the sampled radius;
- extreme front/side aspect ratios, which can reveal flat or accidental column-like shapes;
- occupied row/column counts and center offsets, which make sparse branches, hanging accents, and off-center roots easier to compare;
- sample margins and top headroom, which show whether the contact sheet clipped the object or simply chose a tight view;
- dominant material share near `100%`, which can indicate missing accent or trim materials;
- unexpected bounds, which can mean the selected root is not centered on the visible landmark.

Keep this tooling isolated. Do not adjust LOD, renderer, or procedural generator files just to improve object-lab output; change those systems only when the visual issue is confirmed to be caused by runtime logic rather than the lab harness.
