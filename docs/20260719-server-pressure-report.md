# Server pressure and concurrency report — 2026-07-19

## Scope

These runs exercise real native VXWP clients, movement, collision-critical chunk requests,
presence, edits, persistence, bandwidth accounting, and the optimized Rust world service on an
Apple M3 Max with 16 logical cores. A separate `worldgen-dev` daemon from an interactive `vp dev`
session remained active throughout, so the results represent realistic local contention rather
than an isolated machine.

All population comparisons used:

```sh
vp run automation -- run bot-load \
  --counts=128,256,512,1000 --duration=5 --no-browser \
  --service-profile=worldgen --bot-profile=worldgen \
  --generation-workers=8
```

CPU percentages follow `ps`: 100% is one fully occupied logical core. Chunk and edit columns are
the p95 of the clients' own p95 latency measurements. The five-second stages emphasize connection
and initial-world pressure; they are not long-duration soak tests.

## Procedural world before and after presence scheduling

The baseline was commit `7e9f3a5615be`; the optimized run was commit `4f04a54`. Both used the same
seed, mixed behavior roster, optimized binaries, eight generation workers, and fresh isolated
worlds.

| Clients | Server CPU p95 before → after | Chunk p95 before → after | Edit p95 before → after | Result                                                          |
| ------: | ----------------------------: | -----------------------: | ----------------------: | --------------------------------------------------------------- |
|     128 |               108.4% → 113.5% |         197.0 → 199.2 ms |         137.1 → 28.4 ms | Latency effectively unchanged; edit result is startup-sensitive |
|     256 |               184.8% → 185.3% |         304.5 → 260.3 ms |          47.0 → 36.0 ms | Chunk p95 −14.5%                                                |
|     512 |               335.1% → 331.3% |         555.4 → 476.7 ms |        116.4 → 107.3 ms | Chunk p95 −14.2%                                                |
|   1,000 |               434.3% → 412.2% |     1,219.4 → 1,130.1 ms |        326.2 → 312.8 ms | Chunk p95 −7.3%; CPU p95 −5.1%                                  |

Every optimized stage connected the requested population, delivered the complete peer roster to
every client, stayed within each client's adaptive bandwidth envelope, and reported zero protocol
violations. At 1,000 clients the service peaked at 1.15 GiB RSS while the load-generator process
itself reached 10.56 logical cores and 2.95 GiB in the baseline sweep. The generator and service
together therefore approach whole-machine CPU saturation; 1,000-client latency is partly a
single-machine test-driver limit.

The native sample identified receiver-specific presence scheduling as the largest server CPU
hotspot, ahead of procedural generation. Replacing ordered-map lookups with hash lookups and
selecting only the best dense-region records without fully sorting every candidate reduced the
existing deterministic 1,000-player presence benchmark from a 2,529 µs median to 1,711 µs
(-32.3%). It still emitted exactly 16 deltas and 80,688 bytes and retained starvation-free delivery
of all 999 peers.

Raw reports:

- Before:
  `target/automation/bot-load/2026-07-19T14-29-51-109Z-26168776/report.json`
- After:
  `target/automation/bot-load/2026-07-19T14-48-52-247Z-ac3f918b/report.json`

## Generation worker sweep

At 256 procedural clients, four/eight/twelve/sixteen workers produced chunk p95 values of
571.4/518.7/554.4/556.9 ms. Eight workers was the best measured point without saturating the host.
At 512 clients the bot process itself consumed roughly ten cores, so higher worker counts traded
host scheduling pressure for noisy latency changes. A reader/writer presence lock and smaller
procedural generation work quanta were also tested and removed: neither improved the client-facing
pressure metrics consistently.

The checked-in default remains eight global generation workers, two ordinary lanes per client, and
one collision-critical lane per client. Collision-critical work still jumps queued ordinary work;
raising the global worker count does not preempt already-running batches and was not a general
latency win.

## Native Metal Terrain Diffusion

The bot harness now enables the service's Metal feature whenever
`--source=terrain-diffusion-30m` is selected. These optimized native Rust/Metal runs all passed with
complete rosters and zero protocol or bandwidth violations:

| Clients | Workers | Server CPU p95 | Server RSS peak |  Chunk p95 | Edit p95 |
| ------: | ------: | -------------: | --------------: | ---------: | -------: |
|     128 |       8 |         106.3% |       171.0 MiB |   163.7 ms |  35.9 ms |
|     512 |       8 |         300.1% |       563.1 MiB |   457.3 ms |  74.5 ms |
|   1,000 |       8 |         378.9% |     1,231.0 MiB | 1,127.1 ms | 299.5 ms |

At 1,000 clients, diffusion and procedural chunk p95 were effectively equal (1,127 versus
1,130 ms). Diffusion used about 8% less service CPU and about 7% more RSS in these short runs.
Single-flight generation and the encoded-product cache successfully prevented nearby clients from
multiplying identical model work.

For 512 diffusion clients, four/eight/sixteen workers measured 498.8/457.3/447.0 ms chunk p95.
Doubling eight to sixteen bought only about 2% lower chunk p95, used more CPU, and worsened edit
latency, so it does not justify changing the shared default.

Raw reports:

- 128:
  `target/automation/bot-load/2026-07-19T15-04-35-994Z-7260c270/report.json`
- 512:
  `target/automation/bot-load/2026-07-19T15-07-52-859Z-3aad54f4/report.json`
- 1,000:
  `target/automation/bot-load/2026-07-19T15-06-22-859Z-4bcbc9dc/report.json`

## Browser observer under pressure

A ten-second stage with 256 native bots plus one Chromium observer rendered all 256 articulated
avatars in one avatar draw call and reported:

- roster ready: 1,012 ms;
- frame p95: 9.2 ms (120 FPS target retained);
- client CPU p95: 3.34 ms;
- GPU p95: 1.90 ms;
- browser errors: none.

The world was not settled by the ten-second deadline: 40 chunks were resident and 453 engine jobs
remained. The stage's overall status was failed because long-running digger bots eventually
attempted edits inside the intentionally protected starting area; those server rejections were
correct, but the bots currently classify them as unexpected. The browser metrics themselves are
valid, while the partial-world result remains a real follow-up for client streaming/scheduling
under dense local load. Increasing server generation workers and splitting generation batches did
not improve it.

Raw report:
`target/automation/bot-load/2026-07-19T14-53-11-567Z-b01edcbb/report.json`.

## Conclusions

1. Keep eight generation workers on this M3 Max. More threads do not provide a good
   latency/throughput tradeoff for either source.
2. The retained server improvement is algorithmic, not a larger thread pool: dense presence work
   is 32% faster in isolation and improves chunk latency by 7–15% at 256–1,000 clients.
3. Native Terrain Diffusion/Metal scales to the full 1,000-client harness without serializing the
   service or violating per-client traffic budgets.
4. The renderer remains within the 120 FPS budget with 256 visible players, but world-settling
   latency under that combined load needs a client-side scheduling benchmark rather than another
   increase in server worker count.
