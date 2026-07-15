# Terrain Diffusion conditioning data

`pipeline-data.json` is the 64-knot ETOPO/WorldClim quantile data published with
`xandergos/terrain-diffusion-30m-onnx` at immutable revision
`ad2df557eca5645f588766101cf3bc3682455c3e`. It is reformatted without changing its JSON values.

- Published-file SHA-256: `e3132c3ef0c65d8613615f9278ffe23bbd9363ddcd87f1cc6f18456bcc9efe5c`
- Checked-in formatted SHA-256: `4e17527829f22435745be8bbbf3427af1e73f7e7031a0e6c339f8edda2dc84bb`

The checked-in hash participates in `WorldSourceIdentity` and is regression-tested. The tables let
the native provider reproduce upstream's quantile-matched synthetic geographic conditioning without
shipping the source ETOPO and WorldClim rasters or requiring raster-processing dependencies at
runtime.
