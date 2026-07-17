# Native bot load baseline — 2026-07-17

This report records the first protocol-faithful mixed-behavior population curve. It was measured on a
MacBook Pro (Mac15,9), Apple M3 Max with 16 CPU cores and 128 GB RAM. The daemon used the optimized
`worldgen` profile, the bot driver used `worldgen-dev`, the source was `procedural-v16`, and one
960x540 real Chromium/WebGPU observer joined each fresh stage.

The command was:

```sh
vp run bench:bots -- --duration=5
```

Each population used a fresh temporary database, so rows compare isolated load rather than cumulative
history. CPU percentages use `ps` semantics; 100% is one logical core. Down/up are aggregate delivered
TCP stream bytes for all bots and the observer. The five-second interval is a short saturation probe,
not a capacity promise or latency percentile with production-grade statistical confidence.

## Population curve

| Bots | Server CPU p95 | Server RSS peak | Bot CPU p95 |      TCP down/up |   DB growth | Accepted edits | Mutations | Chunk p95 | Edit p95 | Observer LOD by 5 s     | Frame p95 |
| ---: | -------------: | --------------: | ----------: | ---------------: | ----------: | -------------: | --------: | --------: | -------: | ----------------------- | --------: |
|    4 |         100.3% |        73.4 MiB |        7.4% |  6.64 / 0.08 MiB | 1,098.4 KiB |             30 |       597 |   22.9 ms |   7.8 ms | Full at 3,059 ms        |   10.3 ms |
|    8 |         106.8% |        88.2 MiB |        9.3% |  8.03 / 0.12 MiB | 2,434.2 KiB |             60 |     1,290 |   24.4 ms |   4.6 ms | Full at 3,045 ms        |   10.3 ms |
|   16 |         115.9% |       105.8 MiB |       19.2% | 12.04 / 0.22 MiB | 4,255.1 KiB |            120 |     2,689 |   27.9 ms |   6.2 ms | Full at 3,545 ms        |   15.6 ms |
|   32 |         134.5% |       145.2 MiB |       39.4% | 21.94 / 0.41 MiB | 4,555.3 KiB |            224 |     5,343 |   44.4 ms |   4.7 ms | Interactive at 4,575 ms |   10.4 ms |
|   64 |         177.9% |       168.5 MiB |       77.0% | 48.74 / 0.77 MiB | 5,179.6 KiB |            433 |    10,383 |   76.5 ms |  15.4 ms | Partial, 230 resident   |    9.9 ms |

All stages passed with the exact expected roster, zero rejected edits, zero resyncs, and no browser,
protocol, or WebGPU errors. The observer acquired every bot in the first 500 ms sampling interval.
At 64 bots its frame maximum was 10.4 ms, renderer CPU p95 was 4.43 ms, and GPU p95 was 2.95 ms, but
the world still had 241 pending jobs and only 26 visible chunks at the end. The LOD column is therefore
essential: the 64-bot frame result must not be compared with a fully settled scene as though geometry
were identical.

The 64-bot daemon used 1.78 logical cores at p95 and peaked at 168.5 MiB RSS. The single native bot
driver peaked at 229.1 MiB RSS while holding 64 independent chunk caches and protocol sessions. This
driver memory is test infrastructure, not server memory.

## Bandwidth attribution at 64 bots

The stage delivered 48.59 MiB of downstream VXWP payload in five seconds. The largest message classes
were:

| Downstream message   |   Payload |
| -------------------- | --------: |
| Presence deltas      | 18.23 MiB |
| Edit commits         | 15.09 MiB |
| Surface tile results | 14.12 MiB |
| Chunk results        |  1.10 MiB |

This is an intentionally dense edit workload: 16 diggers, 16 builders, and 16 followers generated
about 87 accepted actions/s, and commits were replicated to interested nearby clients. The aggregate
downstream rate was 9.75 MiB/s, about 1.20 Mibit/s (1.26 decimal Mbit/s) per connected client when
divided across the 64 bots and one observer. Presence and edit fan-out, not canonical
chunk payloads, are the next bandwidth target for very dense crowds. Distant groups remain isolated
by the existing spatial interest index; this run does not claim a 64-player dense result generalizes
to hundreds of mutually distant players.

## Durable world growth

The fresh 64-bot database ended with:

- 65 players including the browser observer and 975 inventory rows;
- 10,324 live sparse voxel overrides;
- 433 accepted edit operations and 10,383 operation-mutation rows;
- 5.1 MiB of physical database/WAL/SHM growth.

Terrain discovery itself wrote no generated chunks to disk. The physical total is shaped by SQLite
WAL checkpoints, so it plateaus between some population rows even as logical history grows.

A separate persistent native-only growth probe used:

```sh
vp run bench:bots -- --counts=4,8,16 --duration=3 --growth --no-browser
```

It passed with zero rejections. The same stable players resumed between waves; cumulative live voxel
overrides grew from 558 to 1,629 to 3,762, while physical peak size grew from 0.79 to 2.52 to
4.30 MiB. Returning builders continued above their authoritative existing tower columns rather than
replaying placements into occupied voxels.

## Improvements made from the measurements

Dense 0.5 m edits originally acquired the global presence state and scanned recipients once for every
mutation. The server now deduplicates affected X/Z columns and spatial cells, locks once, and evaluates
each candidate player once against the exact union. A focused 64-player run reduced edit p95 from
20.4 ms to 10.5 ms while accepted work rose from 270 to 434 actions in the same five-second window.
The final full curve measured 15.4 ms at 433 actions, illustrating the variance of these short
machine-local samples while retaining the larger throughput gain.

The first 64-player browser run also exposed an avatar-shadow uniform mismatch that invalidated the
WebGPU pipeline only at this scale. The unused shader field was removed, the Rust/WGSL size contract
is asserted at 80 bytes, and the final curve has no WebGPU validation errors.

The next high-value scaling work is message aggregation or a more compact mutation representation for
dense edit fan-out. Cloning the full in-memory edit overlay for each accepted operation is also a
longer-term server CPU concern. Both changes touch ordering, idempotency, resync, and interest
correctness, so they should be benchmarked with this harness rather than introduced as an unmeasured
shortcut.
