# 1,000-player traffic shaping report — 2026-07-17

This report measures the native server and protocol-faithful Rust bot harness on a MacBook Pro
(Mac15,9), Apple M3 Max with 16 CPU cores and 128 GB RAM. Both the world service and bot driver used
the optimized `worldgen` profile, the source was `procedural-v16`, and the browser observer was
disabled to isolate native server/protocol capacity. All 1,000 bots started in one dense region, so
this is deliberately much harsher than 1,000 players distributed across spatial-interest cells.

The checked-in transport now gives every player one shared token bucket across their world and
presence WebSockets:

- sustained VXWP payload: 98,304 bytes/s (0.786 decimal Mbit/s);
- startup burst: 65,536 bytes;
- one encoded frame larger than the burst is sent whole, then its entire debt is repaid;
- control, collision/startup data, world changes, presence, visible terrain, and prefetch are
  selected in decreasing priority with weighted, work-conserving service.

The writers register every non-empty traffic class with the shared scheduler. This matters: choosing
one strict-priority frame before entering the scheduler hid lower classes and could starve edits.
Presence input also has a dedicated bounded reader, so outbound replication pressure cannot turn
normally spaced movement updates into an authority-breaking burst.

## Same-duration before/after

Both rows used:

```sh
vp run bench:bots -- --counts=1000 --duration=3 --no-browser \
  --service-profile=worldgen --bot-profile=worldgen
```

| Metric                      |      Unshaped |  Final shaped |    Change |
| --------------------------- | ------------: | ------------: | --------: |
| Aggregate downstream VXWP   |    366.44 MiB |    337.80 MiB |     -7.8% |
| Server CPU p95              |        387.1% |        414.6% |     +7.1% |
| Server RSS peak             |     633.7 MiB |     849.5 MiB |    +34.1% |
| Collision/startup chunk p95 |    2,850.6 ms |    2,056.3 ms |    -27.9% |
| Chunk results received      |        15,039 |        30,687 |   +104.1% |
| Edit p95                    |      339.5 ms |      658.9 ms |    +94.1% |
| Accepted edits              |           398 |           411 |     +3.3% |
| Complete per-client rosters | 1,000 / 1,000 | 1,000 / 1,000 | unchanged |
| Resyncs / protocol errors   |         0 / 0 |         0 / 0 | unchanged |

The three-second sample is dominated by startup credit, so it demonstrates prioritization more than
the sustained rate ceiling. CPU and memory did not improve at 1,000: the shaped run completed more
than twice as many requested chunk products instead of leaving them pending. That is useful work and
improves startup latency, but it also fills more of the bounded product cache and per-session request
windows. The edit latency regression is the tradeoff for guaranteeing collision/startup progress
during this short all-dense burst.

## Sustained result

The final sustained command was:

```sh
vp run bench:bots -- --counts=1000 --duration=10 --no-browser \
  --service-profile=worldgen --bot-profile=worldgen
```

| Metric                                       |                      Result |
| -------------------------------------------- | --------------------------: |
| Connected clients                            |                       1,000 |
| Visible peers per client                     |                         999 |
| Server CPU p95                               |                      484.1% |
| Server RSS peak                              |                 1,267.3 MiB |
| Bot-driver CPU p95                           |                      530.1% |
| Aggregate downstream VXWP                    |                  975.73 MiB |
| Per-client downstream p95 / max              |        0.810 / 0.814 Mbit/s |
| Configured sustained cap / burst             |       0.786 Mbit/s / 64 KiB |
| Budget-envelope violations                   |                           0 |
| Presence / edits / visible world             | 590.48 / 359.69 / 23.69 MiB |
| Accepted edits / expected occupied conflicts |                 4,671 / 121 |
| Mutations                                    |                      42,539 |
| Collision/startup chunk p95                  |                  2,840.0 ms |
| Edit p95                                     |                    744.5 ms |
| Resyncs / protocol errors / disconnects      |                   0 / 0 / 0 |
| Database growth                              |                 9,139.6 KiB |

The observed short-window rate can exceed 0.786 Mbit/s because the contract deliberately includes
64 KiB of startup credit. The harness checks each client against
`rate × connection time + max(burst, largest frame)` and found no violation. TCP, HTTP upgrade, and
WebSocket framing overhead are reported separately and are not mislabeled as VXWP payload.

## Capacity conclusion

One native process can keep 1,000 co-located protocol clients authoritative and converged on this
machine while enforcing a deterministic per-player bandwidth ceiling. This is a successful
worst-case protocol/interest-management test, not a claim that the dense case is finished:

- roughly five server cores and 1.27 GiB RSS are acceptable for a local stress target but expensive
  for one dense production region;
- 2.84-second chunk p95 and 745-millisecond edit p95 remain visible latency;
- the single native bot process itself consumes several cores, so host-wide CPU is not a pure server
  measurement;
- 1,000 distant players are much cheaper: the deterministic isolated-interest regression produces
  zero entity payload for the observer after its initial frame.

The next high-ROI dense-region work is server-side edit patch coalescing and cheaper presence
candidate ranking. Individual action fan-out cannot remain the long-term representation when
hundreds of players edit the same chunks continuously. The current bounded queue correctly falls
back to canonical resynchronization under overload, but this final run proves the weighted policy
keeps it out of that fallback for the measured workload.
