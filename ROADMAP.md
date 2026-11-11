# Roadmap

| # | Scope | Status |
|---|---|---|
| M0 | Workspace, eframe window, simulation thread and snapshot handoff | done |
| M1 | Simple vessel model, route/heading autopilot, NMEA 0183 over UDP and TCP, overrides, map | done |
| M2 | Physics: gusts and shifts, tide, waves (roll/pitch/heave), sail polars + layline tacking, heel, leeway, soundings + grounding; sensor bias, calibration, latency, resolution, deviation | done |
| M3 | AIS: Class A/B, base stations, AtoNs, SARTs, own ship VDO; target motion and editor; CPA/TCPA; collision-course and random traffic | done |
| M4 | Browser build (wasm), see below | deferred |
| M5 | Serial, WebSocket, TCP client, file recording (TAG blocks) and replay; monitor shows routing | done |
| M6 | NMEA 2000 (28 PGNs incl. AIS, address claim, fast packets) as Yacht Devices RAW, Actisense ASCII and canboat plain; Signal K deltas with hello and discovery; one data format per output | done |
| M7 | Fault injection, scenario library, headless CI scenarios | |
| M8 | MCP server: AI agents watch and drive the simulation (app: Streamable HTTP on localhost; CLI: HTTP or stdio) | done |

## M4: browser build (wasm), deferred

On hold (user decision, 2026-11-08); the plan below is kept for later.

Run the simulator as a static web page (eframe `WebRunner`, built with Trunk).

- Engine: replace the 10 Hz thread with a `step()` driven from the UI frame loop
  (`request_repaint_after`); `web-time` instead of `std::time::Instant`; chrono `wasmbind`;
  getrandom `wasm_js` backend.
- Outputs behind a trait: native keeps `transports` (tokio UDP/TCP/serial); the browser gets a
  WebSocket client (`ewebsock`), Web Serial (Chromium) and a recorded-file download. A small
  native relay (the CLI) can re-emit WebSocket data on TCP/UDP 10113.
- Files: open via rfd's async picker, save scenarios as downloads; settings in eframe
  persistence; built-in scenarios and polars bundled.
- Limits: no listening sockets, background tabs throttle timers, Web Serial is Chromium only.
- Deploy `dist/` as static files; CI job builds `wasm32-unknown-unknown` to keep it compiling.

## Ideas collected for later milestones

From a review of the data an existing NMEA tool exposes (data parameters only; look and feel not copied):

- **AIS, later:** message 8/6 binary payloads (e.g. meteorological data), message 27
  (long range), message 9 (SAR aircraft), interrogation/assigned-mode behaviour, slot-level
  timing, and AIS receive (decode targets from a live feed into the simulation).
- **Multiple instances of a sensor type (later):** e.g. GPS1/GPS2 with different talkers and
  noise, each routed to a different output.
- **Engine data:** oil pressure and temperature, coolant, fuel rate (RPM done in M2; NMEA 2000
  PGN 127489 once the engine model has them). Heave is sent as PGN 127252 since M6.
- **NMEA 2000 and Signal K, later:** replay of NMEA 2000 and Signal K logs; canboat JSON
  (decoded fields) output; the Signal K REST API (full data model), so Signal K clients can
  connect without a server; receiving NMEA 2000 requests (ISO request 59904).
- **Radar simulator (to plan):** synthetic radar from the scenario (coastline/targets/AIS
  vessels as echoes, range/bearing noise, clutter); ARPA target output (`TTM`, `TLL`, `OSD`,
  `RSD`) over NMEA 0183, and possibly a raw spoke stream for radar display software.
- **Manual steering (M7):** rudder/throttle control instead of the autopilot.
