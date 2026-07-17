# Adaptive backpressure report

Date: 2026-07-17

VXWP v19 replaces the fixed per-client application rate with a bounded, receiver-feedback
controller. TCP still owns congestion control. The VXWP layer decides how much reliable world work
may enter the transport and which class is allowed to enter next.

The design follows three useful constraints from the literature:

- interactive real-time traffic should seek useful bandwidth without maintaining a standing queue
  ([RFC 8836](https://www.rfc-editor.org/rfc/rfc8836.html));
- queue delay is a better overload signal than queue length
  ([RFC 8289](https://www.rfc-editor.org/rfc/rfc8289.html)); and
- model-based controllers separate a path's propagation RTT from excess delay and probe available
  delivery capacity rather than assuming one static link
  ([BBR congestion-control draft](https://datatracker.ietf.org/doc/draft-ietf-ccwg-bbr/02/)).

## Checked-in controller

- A client starts at the configured 96 KiB/s safe floor and can reach a 4 MiB/s ceiling.
- The browser returns its previous receiver-observed RTT every 250 ms. A client that stops returning
  feedback returns to the floor after three seconds.
- The server retains the minimum RTT, detects both excess delay and a rising RTT trend, increases
  the rate by 15% only while work is currently waiting, and halves the rate on congestion.
- The effective token burst is the smaller of the configured 64 KiB cap and one selected-rate
  queue-delay target, with an 8 KiB minimum pacing quantum.
- A VXWP message above 32 KiB is split into independently scheduled 8-32 KiB fragments. Critical,
  collision, edit, presence, visible-world, and background-world traffic therefore compete between
  fragments rather than behind an indivisible terrain product.
- Far-surface requests use two-tile batches with four batches in flight. This keeps compression and
  generation concurrent without recreating a single large WebSocket write.

All limits are server configuration except the versioned browser feedback cadence. The world and
presence sockets share one controller, so a second socket cannot evade the byte budget.

## Standalone Chromium results

These are deterministic one-run diagnostics on the same machine, not statistical WAN claims.
`viewport` is time until the presented-geometry fingerprint equals the final settled viewport.
Queue delay excludes configured propagation and the pacing quantum's own serialization. Bytes
include HTTP/WebSocket framing but exclude TCP/IP/TLS overhead.

### Good remote link: 40 ms RTT, 50/10 Mbit/s

| Scenario          | Before viewport | VXWP v19 viewport | Before peak queue | VXWP v19 peak queue | Before pauses | VXWP v19 pauses |
| ----------------- | --------------: | ----------------: | ----------------: | ------------------: | ------------: | --------------: |
| Cold spawn        |       18,616 ms |         11,216 ms |           86.2 ms |             10.5 ms |             9 |               0 |
| Turn during spawn |        7,235 ms |          9,022 ms |           88.8 ms |              9.2 ms |             9 |               0 |

The selected application rate rose from the 0.79 Mbit/s floor to 25.9 Mbit/s during cold spawn and
29.8 Mbit/s during the warm-cache pivot. Cold-spawn world traffic at the informed viewport was
9,404,174 bytes. Main-thread frame p95 was 10.3 ms with no frame above 33.33 ms.

The before run is
`target/network-benchmark/2026-07-17T14-41-20.960Z.json`; the checked-in result is
`target/network-benchmark/2026-07-17T22-24-34.459Z.json`.

### Constrained link: 80 ms RTT, 10/2 Mbit/s

| Scenario          | Before viewport | VXWP v19 viewport | Before peak queue | VXWP v19 peak queue | Before pauses | VXWP v19 pauses |
| ----------------- | --------------: | ----------------: | ----------------: | ------------------: | ------------: | --------------: |
| Cold spawn        |       26,938 ms |         16,377 ms |          137.9 ms |             23.2 ms |             7 |               0 |
| Turn during spawn |       15,154 ms |         24,926 ms |          167.8 ms |             59.0 ms |            22 |               0 |

The warm-cache pivot is deliberately slower: the old controller won throughput by filling the
bounded proxy until it applied source backpressure. VXWP v19 keeps that queue transient and never
hits the hard pause, at the cost of conservative recovery after congestion. Cold spawn improves both
latency and completion time. Streaming movement settled in 1,871 ms with a 13.1 ms peak queue.

The before run is
`target/network-benchmark/2026-07-17T14-50-43.608Z.json`; the checked-in result is
`target/network-benchmark/2026-07-17T22-23-20.610Z.json`.

## Remaining boundary

Receiver RTT is useful flow-control feedback, not an authorization signal. A client can lie, but it
can raise only its own controller to the configured ceiling. A WAN deployment still needs a
process/region egress budget, authenticated quotas, and transport telemetry. WebTransport reliable
streams would later expose native writer backpressure, but they do not remove the need for
application priority, bounded queues, or product fragmentation.
