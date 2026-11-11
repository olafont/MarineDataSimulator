# Marine Data Simulator: user manual

Marine Data Simulator sails a boat that does not exist and sends out the data its instruments
would produce: NMEA 0183, AIS, NMEA 2000 and Signal K, over the network, a serial port or to a
file. Use it to develop and test chart plotters, instrument displays, hubs and alarms without
leaving the dock. Nobody gets wet, and the only thing that can run aground is a triangle on a
chart.

This manual covers the desktop app, the command-line runner and the scenario file format. If
you are new, read sections 1 to 4: they take you from installing the app to steering the boat.
The rest is reference, to open when you need it.

> **For testing only: never on a vessel under way, never into public AIS services.**
> See [Safety and responsible use](#safety-and-responsible-use).

**Contents**

1. [Getting started](#1-getting-started)
2. [Concepts](#2-concepts)
3. [The main window](#3-the-main-window)
4. [Steering the vessel](#4-steering-the-vessel)
5. [Vessel, environment and dynamics](#5-vessel-environment-and-dynamics)
6. [Overrides](#6-overrides)
7. [Sensors](#7-sensors)
8. [AIS](#8-ais)
9. [Outputs, recording and replay](#9-outputs-recording-and-replay)
10. [Preferences and saved settings](#10-preferences-and-saved-settings)
11. [Command line (CLI)](#11-command-line-cli)
12. [Scenario files](#12-scenario-files)
13. [Connecting other software](#13-connecting-other-software)
14. [AI agents (MCP)](#14-ai-agents-mcp)
15. [Troubleshooting](#15-troubleshooting)

The names of menus and buttons are written here as the English interface shows them. The app
also speaks French and German ([section 10](#10-preferences-and-saved-settings)).

---

## 1. Getting started

### Install

**macOS.** Open the disk image (`MarineDataSimulator-<version>-macos-<arch>.dmg`) and drag the
app to Applications. To build the app and the disk image yourself:

```sh
scripts/build-macos-app.sh               # this Mac's architecture
scripts/build-macos-app.sh --universal   # Apple Silicon and Intel
scripts/build-macos-app.sh --no-dmg      # the app only, no disk image
```

They are written to `target/macos/`.

The app is signed *ad hoc*, without an Apple Developer ID, so macOS blocks a copy that comes
from another Mac or a download the first time you open it:

1. Open the app. macOS says it cannot verify the developer; click **Done**.
2. Go to **System Settings → Privacy & Security** and click **Open Anyway** next to
   *Marine Data Simulator*, then confirm.

Or, from a terminal: `xattr -dr com.apple.quarantine "/Applications/Marine Data Simulator.app"`.
macOS may also ask whether the app may accept incoming network connections. Allow it if other
computers should reach its TCP or WebSocket outputs.

**Windows and Linux**, or to run from source: see the [README](../README.md#quick-start)
(`cargo run --release -p app`). To start on a scenario of your own, give its file as the first
argument: `cargo run --release -p app -- my-scenario.toml`.

### Safety and responsible use

Marine Data Simulator produces **simulated** data for testing. It is not for navigation.

- **Never connect it to the instruments or network of a vessel that is under way, or to a
  system someone relies on for navigation.** Simulated positions, depths and AIS targets look
  real on a chart plotter, and could cause an accident.
- **Do not send its AIS output to public AIS services** (MarineTraffic, AISHub, VesselFinder
  and the like) or to a VHF transmitter. Feeding fictitious vessels into real AIS traffic is
  spoofing: it breaks those services' terms and, in many countries, the law. The simulator
  itself never transmits on radio.
- **Fictitious identities.** The built-in scenarios, and the targets you add in the app, use
  MMSIs that cannot belong to a real station: where a real MMSI has a country code (201 to
  775), they have 100. Ship names, call signs and IMO numbers are made up (the IMO numbers fail
  their check digit). If you write your own scenarios, do not use the MMSI or name of a real
  vessel.
- **On a shared network**, outputs on `0.0.0.0` or to a broadcast address reach every computer
  on it, including any navigation equipment connected to it. Prefer `127.0.0.1` or a network
  set aside for testing.

The software comes with no warranty (GPL-3.0, sections 15 and 16).

### First run

The app opens on the built-in **Cape Canaveral** scenario: a 12 m sailing yacht following a loop
of waypoints off Cocoa Beach, with AIS traffic around it. It casts off at once and starts
sending NMEA 0183 on two outputs:

- **TCP 10113**: a TCP server on port 10113. It listens on every network interface
  (`0.0.0.0`), so other computers on your network can connect too.
- **UDP localhost 10113**: UDP datagrams to `127.0.0.1:10113`.

Port 10113 is used instead of the usual NMEA port 10110, so the simulator does not clash with
other NMEA software on the same computer.

To see the data arrive somewhere, connect a chart plotter. In OpenCPN, for example:
*Options → Connections → Add*, Network, TCP, address `localhost`, port `10113`. The vessel and
the AIS targets appear on its chart. [Section 13](#13-connecting-other-software) covers other
software.

### A five-minute sea trial

1. **Watch the data.** The **Traffic** monitor at the bottom shows every line as it is sent.
   Click **❄ Freeze** to read it at leisure.
2. **Take the helm.** On the **Home** tab, click **Heading**, then change the **Heading** value
   in the **Navigation** section on the left. Click **Route** to hand the boat back to her
   waypoints.
3. **Speed things up.** Set **Time scale** to 10× and watch the boat tack round her loop.
   **Real time** brings you back to 1×.
4. **Order some weather.** In **Environment**, change the base value of *True wind speed* and
   see the boat heel and accelerate ([section 5](#5-vessel-environment-and-dynamics)).
5. **Arrange a close encounter.** On the **AIS traffic** tab, click **Collision**: a cargo
   ship sets course for where you will be in 12 minutes ([section 8](#8-ais)). This is the one
   place where that counts as good seamanship.
6. **Add an output** for your own software with **Data › Add output**
   ([section 9](#9-outputs-recording-and-replay)).
7. **Try other waters** from the **Scenario** tab: Marseille under sail, a motor vessel
   crossing the Channel in a strong tide, a launch off Singapore, a sea-breeze sail off Long
   Beach, or a windy crossing of the Strait of Gibraltar.

If you make a mess, **Reset** puts everything back as the scenario started.

---

## 2. Concepts

The simulator runs a **physics model** of one vessel (your *own ship*) in an **environment**
(wind, current, tide, waves, depth, weather). The model is updated about ten times a second,
in smaller steps when simulated time runs fast.

Data flows through these stages:

```
environment ─► physics ─► instruments ─► sensors ─► encoders ─► outputs
 (inputs)                 (true values)   (errors,   (NMEA 0183,  (UDP, TCP,
                                           rates)     N2K, SK)     serial, …)
```

- **Environment**: the conditions acting on the vessel. You set *base values*; the dynamics
  add gusts, wind shifts, tide, soundings and a pressure trend to give the *live* values.
- **Instruments** (also called channels): the true values the physics produces, such as
  position, heading, speed through water, apparent wind and depth.
- **Sensors**: each one reads instrument values at its own rate and adds its own errors, as a
  real sensor does.
- **Encoders** turn each reading into NMEA 0183 sentences, NMEA 2000 PGNs and Signal K deltas.
- **Outputs** send one data format each to the network, a serial port or a file.

You can **override** any environment or instrument value ([section 6](#6-overrides)), which is
how you test what your software does when the depth sounder lies.

A **scenario** holds everything about a run: vessel, environment, sensors, AIS traffic and
outputs. Everything you can do in the app can be written in a scenario file (TOML,
[section 12](#12-scenario-files)), and **Save as…** writes the current state to one.

Two things belong to this computer and not to the scenario: your preferences, and the outputs
you add yourself ([section 10](#10-preferences-and-saved-settings)).

---

## 3. The main window

| Area | Contents |
|---|---|
| Title bar (top) | On macOS: quick commands, the ribbon's tabs, the status and **⚙** Preferences |
| Ribbon | The commands of the current tab |
| Left panel | **Navigation**, **Vessel**, **AIS**, **Environment**, **Dynamics (gusts, tide)**, **Instruments** |
| Chart (centre) | OpenStreetMap with OpenSeaMap seamarks; your vessel, track, route, AIS targets, wind and current |
| Right panel | **Outputs**, **Replay**, **Sensors** |
| Traffic monitor (bottom) | The latest lines sent, with their source and destinations |

Drag the edge of a panel to resize it, and click a section's title to fold it away. The
**View** tab hides or shows the side panels and the traffic monitor.

Most numbers in the panels can be changed in two ways: drag the value sideways, or click it
and type.

### Title bar and status

On **macOS** the app draws its own title bar, next to the window's close, minimize and zoom
buttons, so the ribbon's tabs need no row of their own. From left to right:

- **📂** opens a scenario and **💾** saves the current state as one.
- **⏸ / ▶** pauses or runs the simulation; **⟲** resets to the scenario's initial state.
- The ribbon's **tabs**.
- The **status**: the scenario's name (or file name), the simulated date and time in UTC, and
  the number of lines sent per second. **PAUSED** and **AGROUND** join it when they apply.
- **⚙** opens Preferences; **⏶ / ⏷** hides or shows the ribbon.

Drag empty space in the bar to move the window; double-click it to zoom. On **Windows and
Linux** the system's title bar is kept, and the tabs and the status sit at the top of the
ribbon.

Two things to know about the status:

- **AGROUND** appears when the water depth is less than the vessel's draft. The vessel stops,
  and the chart circles her. Steer towards deeper water or move the vessel. No tow, no
  paperwork.
- Sensor rates follow simulated time, so at 10× a 1 Hz GPS sends 10 sentences per real second.
  Outputs stay open while the simulation is paused.

### Ribbon

The ribbon gathers the commands by task. Each one does the same as a control in the panels, so
use whichever is closer to hand. Click the current tab (or **⏶**) to hide the commands and
keep only the tabs; click any tab to show them again.

| Tab | Group | Commands |
|---|---|---|
| **Home** | Simulation | **Run / Pause**, **Reset**, **Time scale** (0.1× to 120×), **Real time** (back to 1×) |
| | Steering | **Heading**, **Wind angle**, **Route** |
| | Propulsion | **Engine** on or off (sailing vessels only) |
| **Scenario** | File | **Open…** a scenario file, **Save as…** the current state, **Reset** to the scenario's initial state |
| | Built-in scenarios | **Scenario** list: *Cape Canaveral sailing loop*, *Marseille sailing loop*, *Channel crossing (motor, tidal)*, *Singapore Strait (motor)*, *Long Beach sailing loop*, *Strait of Gibraltar crossing*. Choosing one loads it. |
| **Chart** | Route | **Add waypoints** by clicking the chart, **Clear route** |
| | Display | **Seamarks** overlay on or off, **Clear track** |
| **AIS traffic** | Add | The **Class** of new targets; add a **Target**, a **Collision** course target or **Random** traffic |
| | Targets | **Remove all**, and the number of targets |
| **Data** | Outputs | **Add output**, **Record…** to a file |
| | Replay | **Open log…**, **Play / Pause**, **Start** (back to the start of the log), **Close** |
| | Traffic monitor | **Freeze** |
| **View** | Panels | **Vessel and environment** (left), **Outputs and sensors** (right), **Traffic monitor** (bottom) |
| | Palette | **Day**, **Dusk**, **Night** |
| | Settings | **Preferences…** |

A group whose caption ends in **⏷** has more to offer: click the caption to open a menu with
its less common commands and settings.

| Group | Its ⏷ menu |
|---|---|
| Home › Simulation | Every time scale |
| Home › Steering | The target speed, and the heading or wind angle being held (or the route's progress) |
| Chart › Route | Number of waypoints, **Loop**, **Arrival radius** |
| AIS traffic › Add | **Collision course** (time to meet, relative bearing, speed) and **Random traffic** (number of targets, radius) |
| AIS traffic › Targets | Every target; click one to select it and edit it in the AIS section |
| Data › Outputs | Every output with its switch and state (hover over the dot for details) |
| Data › Replay | Replay speed, **Loop**, **Mute simulated data** |
| Data › Traffic monitor | The data format the monitor shows |

**Palettes**, as on a bridge display: **Day** is light; **Dusk** is dark and dims the chart;
**Night** is dim red on black and darkens the chart further, to keep your eyes adapted to the
dark, should you be testing at three in the morning. Until you pick one, the app follows the
system's light or dark mode. Your choice is remembered.

### Chart

| Action | How |
|---|---|
| Pan | Drag |
| Zoom | Scroll wheel, or the **+** and **−** buttons at the top left |
| Follow the vessel again after panning | **Follow vessel**, which appears next to **+** once you have panned away |
| Select an AIS target | Click it (the nearest within 14 pixels is selected; click empty water to clear) |
| Add waypoints quickly | Turn on **Chart › Add waypoints**, then click the chart. Turn it off to select targets again. |
| Everything else | Right-click |

The right-click menu shows the position under the pointer and offers:

- **Move vessel here**: moves your vessel there at once and starts a new track.
- **Add waypoint here**: appends a waypoint to your route.
- **Steer towards here**: switches to heading mode, on the bearing to that point.
- **Add AIS target here**: creates a station of the class chosen for new targets. A moving
  target heads across your track.
- When a target is selected, up to three more actions apply to it:
  - **Move *name* here**.
  - **Add waypoint to *name*'s route** (moving targets only).
  - **Send *name* towards here** (moving targets only).

What the chart draws:

- **Your vessel**: a hull outline (at true size when you zoom in), a heading line, and a
  predictor showing where she will be in 6 minutes.
- **Your track** behind her, and **your route** as a dashed line with numbered waypoints.
- **AIS targets**, each with its name. With more than 40 targets, only the selected one is
  named, to keep the chart readable.
- **The rose** at the top right: the wind (solid arrow, pointing downwind), the current and
  the waves, with their values and the tide height below it.
- **A magenta *replay* marker** while a log is loaded: the last position read from the log.

Chart tiles come from the internet; tiles already seen are kept on disk and work offline.

### Traffic monitor

The monitor shows the most recent lines in the format chosen at its left: NMEA 0183, one of
the NMEA 2000 formats or Signal K. It can show a format even when no output uses it. (An AI
agent that reads another format changes this choice: [section 14](#14-ai-agents-mcp).) Each line
has three columns:

1. The **source**: the sensor that produced it, *Vessel* (the Signal K identity) or *Replay*.
2. **»N**: the number of outputs that send it. Hover over it to see their names. **»0** means no
   output accepts the line, so it goes nowhere.
3. The line itself.

Type in the **filter** box to show only matching lines, e.g. `RMC`, `$WI`, `VDM` or a PGN such
as `129025`. **Show** sets how many lines are kept on screen (300 at first, 50 to 2000).
**❄ Freeze** stops the display so you can read it; the simulation carries on underneath.

---

## 4. Steering the vessel

Steer from the ribbon's **Home** tab, or from the **Navigation** section of the left panel,
which also holds the route. Navigation has these rows:

| Row | Meaning |
|---|---|
| **Under** | Sailing vessels only: **⛵ Sail** or **⚙ Engine**. Under engine the vessel motors at the target speed. A motor vessel has no choice in the matter. |
| **Steering** | **Heading**, **Wind angle** or **Route** (below) |
| **Target speed** | The speed through water to hold under engine. While a sailing vessel is under sail the row is called **Engine speed target**: it applies once the engine is on. |
| **Heading** / **True wind angle** | The value to hold in those two modes |
| **Tack** | Sailing on a route: the current tack, *port* or *starboard* |

The three steering modes:

| Mode | Behaviour |
|---|---|
| **Heading** | Holds a true heading. Switching to it starts from the current heading. |
| **Wind angle** | Holds a true wind angle (positive = wind from starboard), starting from the current one. The heading follows the wind shifts, like a wind-vane self-steering gear. |
| **Route** | Follows the waypoints and corrects for current and leeway. A sailing vessel tacks on the laylines when the next waypoint is upwind. |

**Routes**

The route's controls appear under Navigation while steering by **Route**.

- **Add waypoints** by right-clicking the chart, or turn on **+ Add on click** (the same switch
  as **Chart › Add waypoints**) and click the chart.
- **The list** shows each waypoint's position and its distance from the vessel; **▶** marks the
  one she is heading for. Click a waypoint to go there next, and **×** to remove it. **Clear**
  removes them all.
- **Arrival** sets the radius at which a waypoint counts as reached (5 to 2000 m).
- With **Loop** on, the route repeats. Without it, the vessel stops at the last waypoint and
  drifts with the current, and the **Status** row shows *Route complete: stopped, drifting*.
  Click a waypoint to set off again.
- Switching to **Heading** or **Wind angle** keeps the route; switch back to **Route** to pick
  it up where you left it.

---

## 5. Vessel, environment and dynamics

### Vessel

The **Vessel** section describes the boat herself.

| Setting | Meaning |
|---|---|
| **Propulsion** | **Motor** or **Sail** |
| **Polar** (sail) | The *built-in cruiser* (a 12 m cruising yacht), or your own file: click **Load…**. **Built-in** goes back to the default. |
| **Optimum angles** (sail) | The best upwind (*beat*) and downwind (*run*) angles for the current wind |
| **Polar efficiency** (sail) | Scales the polar's speeds: 1 sails to the polar, less for a foul bottom and tired sails |
| **Heel coefficient**, **Max heel** (sail) | How much she heels to the wind, and the limit |
| **Leeway coefficient** (sail) | How much she slips sideways |
| **Max engine speed** | Top speed under engine; also the limit of the target speed |
| **Length**, **Beam**, **Draft** | Dimensions. The draft decides when she runs aground. |
| **Max rate of turn** | How quickly she can change heading |
| **Speed time constant** | How quickly she gains or loses speed |
| **Roll period** | The period of her rolling in waves |
| **Idle rpm**, **Max rpm** | The range of the engine speed reported |

**Polar file format:** CSV or TSV (separated by `;`, `,` or tabs; lines starting with `#`
are ignored), with the extension `.pol`, `.csv` or `.txt`. The first row lists true wind
speeds in knots, in increasing order. Each following row starts with a true wind angle,
followed by the boat speed for each wind speed.

```
twa/tws;6;8;10;12;14;16;20;25
42;3.6;4.6;5.3;5.8;6.1;6.3;6.4;6.3
90;5.2;6.2;6.9;7.2;7.5;7.8;8.1;8.3
180;3.1;4.0;4.9;5.7;6.3;6.8;7.7;8.5
```

### Environment

Here you are the weather. Each row of the **Environment** section shows:

- the **base** value, which you set;
- the **live** value: the base plus the dynamics (gusts, tide, soundings), or the override,
  shown highlighted;
- the **override** controls ([section 6](#6-overrides)).

| Row | Unit | Key in scenario files |
|---|---|---|
| True wind speed | kn | `wind_speed` |
| True wind direction (from) | °T | `wind_direction` |
| Current speed (drift) | kn | `current_speed` |
| Current set (towards) | °T | `current_set` |
| Water depth (chart + tide) | m | `seabed_depth` |
| Water temperature | °C | `water_temp` |
| Air temperature | °C | `air_temp` |
| Barometric pressure | hPa | `pressure` |
| Relative humidity | % | `humidity` |
| Magnetic variation (+E) | ° | `magnetic_variation` |
| Significant wave height | m | `wave_height` |
| Wave peak period | s | `wave_period` |
| Wave direction (from) | °T | `wave_direction` |

Mind the sailor's convention: wind and waves are named for where they come *from*, the
current for where it goes *to*.

### Dynamics (gusts, tide)

Real weather never holds still, and neither does this one.

- **Gusts (std dev)** and **Gust period**: how much the wind speed varies, as a percentage of
  its mean, and how fast.
- **Shifts (std dev)** and **Shift period**: how much the wind direction wanders, in degrees,
  and how fast.
- **Pressure trend**: in hPa per hour. A falling glass, if you want one.
- **Tide**: tick **enabled** for a semi-diurnal tide.
  - **Range** and **Period** (12.42 h by default).
  - **Flood sets towards** and **Max stream**: the tidal stream floods in that direction at up
    to that rate, and ebbs the opposite way.
  - **Next high water in**: hours until the next high water.
  - **Now** shows the current height and stream.

**Water depth** comes from soundings when the scenario has them. The depth at any point is
interpolated between the nearest soundings (4 by default), and the tide height is added.
Without soundings, the *Water depth* row applies everywhere.

---

## 6. Overrides

Any row of **Environment** and any row of **Instruments** can be overridden: tick the box at
the end of the row, then choose a mode.

| Mode | Effect |
|---|---|
| **Replace** | The value becomes the number you enter. |
| **Offset** | Your number is added to the modelled value. |
| **Freeze** | The value stays at what it was when you froze it. |

Overridden values are highlighted. Hover over an overridden instrument to see the modelled
value behind it. Untick the box to go back to the model.

The two sections behave differently, and the difference matters:

**Environment overrides change the physics.** Replace *True wind speed* with 30 and the boat
heels, speeds up and makes more leeway. The sea really is rougher.

**Instrument overrides change only what is reported.** The vessel keeps moving as before; her
instruments just stop telling the truth. Use them to test how receiving software copes with
bad data:

- Replace *Depth below surface* with 2.0 to set off a shallow-water alarm.
- Freeze *Latitude* and *Longitude* to simulate a stuck GPS.
- Offset *Heading* by 15 to simulate a badly swung compass.
- Replace *Apparent wind speed* with 0 to simulate a wind sensor that a seagull sat on.

The **Instruments** rows, with their keys for scenario files:

| Row | Key | Row | Key |
|---|---|---|---|
| Latitude | `latitude` | Speed over ground | `sog` |
| Longitude | `longitude` | Course over ground | `cog` |
| Heading | `heading_true` | Depth below surface | `depth` |
| Magnetic variation (+E) | `magnetic_variation` | True wind speed | `tws` |
| Rate of turn | `rate_of_turn` | True wind angle | `twa` |
| Rudder angle (+stbd) | `rudder` | True wind direction | `twd` |
| Roll / heel (+stbd down) | `roll` | Apparent wind speed | `aws` |
| Pitch (+bow up) | `pitch` | Apparent wind angle | `awa` |
| Heave (+up) | `heave` | Water temperature | `water_temp` |
| Leeway (+to stbd) | `leeway` | Air temperature | `air_temp` |
| Engine speed | `engine_rpm` | Barometric pressure | `pressure` |
| Speed through water | `stw` | Relative humidity | `humidity` |

Overrides also show up in your own ship's AIS reports (`!AIVDO`), because those are built
from the reported values.

---

## 7. Sensors

The **Sensors** section (right panel) lists one sensor of each type, with the NMEA 0183
sentences it produces. Untick a sensor to silence it: a quick way to see what your software
does when the wind instrument goes overboard.

| Sensor | Sentences | Talker | Rate |
|---|---|---|---|
| GPS | RMC GGA GLL VTG ZDA DTM | `GP` | 1 Hz |
| Compass | HDG HDT ROT | `HC` | 10 Hz |
| Speed log | VHW | `VW` | 2 Hz |
| Depth sounder | DBT DPT | `SD` | 1 Hz |
| Wind | MWV MWD | `WI` | 4 Hz |
| Water temperature | MTW | `YX` | 0.5 Hz |
| Weather station | MDA XDR | `WI` | 0.2 Hz |
| Rudder angle | RSA | `AG` | 5 Hz |
| Attitude | XDR (pitch, roll) | `II` | 10 Hz |
| Engine | RPM | `ER` | 2 Hz |
| AIS transponder | VDM VDO (messages 1, 4, 5, 14, 18, 19, 21, 24) | `AI` | per station |

Open a sensor's **settings** to change:

| Setting | Sensors | Effect |
|---|---|---|
| **Rate** | all but AIS | Readings per simulated second (0.01 to 50 Hz) |
| **Talker** | all | The two-letter NMEA talker ID, in capitals (e.g. `GP`, `HC`, `WI`) |
| **Noise** | all | Multiplies the sensor's built-in random noise (0 = a perfect sensor, 1 = normal) |
| **Latency** | all but AIS | Delay between the measurement and the sentence |
| **Resolution** | all but AIS | Rounds readings to this step |
| **Bias** | compass, rudder angle, attitude | A fixed angular error (misalignment) |
| **Vane offset** | wind | The same, for a masthead vane that is not quite straight |
| **Calibration** | speed log, wind | Speed scale factor (1.0 = correct) |
| **Transducer depth** | depth sounder | DBT gives the depth below the transducer; DPT carries the offset |
| **Antenna altitude** | GPS | The altitude in GGA |
| **Deviation** | compass | A + B·sin h + C·cos h + D·sin 2h + E·cos 2h (degrees, h = magnetic heading). HDG reports it; HDT is corrected. |

AIS reporting intervals are set per station, in the AIS section ([section 8](#8-ais)).

True wind is computed from the *measured* apparent wind, speed log and compass, as on a real
instrument system. A speed-log calibration error or a vane offset therefore also shows up in
the true wind, just as it does on a real boat, to the lasting confusion of real navigators.

---

## 8. AIS

The simulator can surround you with traffic: ships, yachts, buoys, shore stations and even a
distress transmitter. Other stations are reported as `!AIVDM`, your own ship as `!AIVDO`.

### Station types

| Class | Messages | Notes |
|---|---|---|
| **Class A** | 1 (position), 5 (static and voyage) | Navigational status, rate of turn, IMO, destination, ETA, draught |
| **Class B** | 18 (position), 24 A and B (static), optional 19 | Class B flags (CS unit, display, DSC, whole band, message 22) |
| **Base station** | 4 | UTC time and position |
| **AtoN** (aid to navigation) | 21 | Type, virtual and off-position flags; a name of up to 34 characters |
| **SART** | 1 with status 14, and 14 "SART ACTIVE" | A search-and-rescue transmitter; MMSI 970xxxxxx |

Vessels and SARTs can also send a **safety text** (message 14), set in the target's editor.
In a scenario file, any station can have one ([section 12](#12-scenario-files)).

Messages alternate between channels A and B. A message longer than one sentence is split
over several sentences.

### Reporting intervals

With **auto** ticked, each station reports as ITU-R M.1371 requires:

| Station | Position report interval |
|---|---|
| Class A, at anchor or moored, ≤ 3 kn | 3 min |
| Class A, ≤ 14 kn | 10 s (3⅓ s when turning) |
| Class A, ≤ 23 kn | 6 s (2 s when turning) |
| Class A, > 23 kn | 2 s |
| Class B, ≤ 2 kn / faster | 3 min / 30 s |
| Base station | 10 s |
| Aid to navigation | 3 min |
| SART | 1 min |

Static data is sent every 6 minutes and safety text every 4 minutes. You can change each
interval per station.

### Adding traffic

The **AIS traffic** tab and the top of the **AIS** section (left panel) offer the same tools:

- **Target** (**Add target** in the section): places a station of the chosen **Class** 2 nm
  from your vessel, in a random direction. To choose the spot yourself, right-click the chart
  and pick **Add AIS target here**. New stations are named *CARGO*, *YACHT*, *BASE* or *BUOY*
  with a number, or *SART ACTIVE*.
- **Collision** (**Collision course**): creates a Class A target that will reach your vessel's
  predicted position. By default it arrives in 12 minutes, from 45° on your starboard bow, at
  14 kn. Change these under **Add ⏷**, or next to the button in the section. Use it to test
  collision alarms.
- **Random** (**Random traffic**): scatters vessels around you, 10 within 6 nm by default. Some
  are Class A, some Class B, and a few lie at anchor.
- **Remove all** (**Remove all targets**) clears the sea.

### Your own transponder

Open **Own ship transponder (VDO)** in the AIS section to switch your own reports on or off
(**Transmit**) and to set the name, MMSI, call sign, class (A or B), navigational status,
static data, flags and intervals.

### The target list

The list shows each station's **Range** and bearing (**Brg**), its **CPA** (closest point of
approach, in nm) and **TCPA** (time to CPA, in minutes; *past* once the target has gone by).
A row turns **red** when the CPA is under 0.5 nm within 30 minutes. Untick a target to stop it
transmitting without deleting it; it also leaves the chart.

### Editing a target

Click a target in the list or on the chart to open its editor under the list. Click it again
in the list to close it. The header shows its speed, course and current reporting interval.

- **Transmitting**, **Name**, **MMSI**, **Class** and **Call sign**.
- **Status** (Class A): the navigational status.
- **Motion** (vessels and SARTs):
  - *Course*: set **COG / SOG**.
  - *Route*: right-click the chart and pick **Add waypoint to *name*'s route**; tick *loop* to
    repeat it.
  - *Stationary*.
- **Max rate of turn**: how quickly the target can change course.
- AtoN only: **Type**, **Virtual** (no physical mark) and **Off position**.
- **Safety text (14)** and, for Class B, **Message 19**.
- **Static data, flags and intervals**: ship type, IMO, destination, ETA, draught, vendor ID,
  antenna position, position device, DTE, the flags (high accuracy, RAIM, assigned mode and the
  Class B flags) and the report intervals.
- **Duplicate** makes a copy 300 m abeam with the next MMSI; **Remove** deletes the target.

On the chart, each class has its own colour and symbol:

- Vessels: triangles pointing along their heading (Class A filled, Class B hollow), with a
  line showing where they will be in 6 minutes.
- Aids to navigation: diamonds, dashed when virtual.
- Base stations: squares.
- SARTs: a circled X.

The selected target is ringed, and its route is drawn.

---

## 9. Outputs, recording and replay

### Outputs

An output is one stream of data leaving the simulator. The **Outputs** section (right panel)
lists each one with its state, where it sends, the lines and kilobytes sent, any errors and,
for servers, the number of connected clients.

| State | Meaning |
|---|---|
| ⚫ running (green) | Sending |
| ⚫ starting (amber) | Opening its port or connecting |
| ⚠ and a message (amber) | Working or retrying, with something you should know (e.g. lines dropped, or still connecting) |
| ⚫ and a message (red) | Cannot send; the message says why |
| ○ off | Switched off |

Tick or untick an output to switch it on or off, click **✏** to edit it and **×** to remove it.

**To add an output**, click **+ Add output** (or **Data › Add output**). The editor opens on a
new UDP broadcast output:

1. Give it a **Name**.
2. Choose its **Type** (table below).
3. Choose its **Data** format.
4. Set where it goes: **Send to** or **Listen on** for network outputs, **Port** and **Baud**
   for a serial port, **File** for a file.
5. Optionally limit it with **Only** (below).
6. Click **Apply**. **Cancel** closes the editor without adding anything.

| Type | Settings | Notes |
|---|---|---|
| UDP | **Send to** `host:port`, **Broadcast** | One line per datagram. For a whole LAN, send to its broadcast address (e.g. `192.168.1.255:10113`) and tick **Broadcast**. |
| TCP server | **Listen on** `addr:port` | `0.0.0.0:10113` accepts connections on every interface; `127.0.0.1:10113` from this computer only. |
| TCP client | **Send to** `host:port` | For a hub that listens for data. Retries every 2 s until it connects. |
| WebSocket server | **Listen on** `addr:port` | One text frame per line. Works with browser apps. |
| Serial port | **Port**, **Baud** | Type the port or pick it from the **…** list. Classic NMEA 0183 uses 4800 baud; AIS and high-speed NMEA use 38400. An unplugged USB adapter is reopened every 2 s. When the traffic is too much for the baud rate, lines are dropped and the output shows a warning. |
| File (record) | **File**, TAG-block timestamps | Appends to the file. With **TAG-block timestamps (for replay)**, every NMEA 0183 line gets a timestamp. A new file output starts in the output folder ([section 10](#10-preferences-and-saved-settings)). |

**Data** sets what the output sends:

| Data | Each line is | Typical use |
|---|---|---|
| NMEA 0183 | a sentence: `$GPRMC,…`, `!AIVDM,…` | chart plotters, OpenCPN, most software |
| NMEA 2000 · Yacht Devices RAW | one CAN frame: `14:30:15.250 R 09F8010A 3F 9F DC FF FF FF FF FF` | anything that reads a YDWG-02 or YDEN-02 gateway (Signal K, OpenCPN, Expedition) |
| NMEA 2000 · Actisense ASCII | one message: `A143015.250 0AFF2 1F801 3F9FDCFFFFFFFFFF` | software that reads an Actisense W2K-1 or NGX in ASCII mode |
| NMEA 2000 · canboat plain | one message: `2026-06-21T14:30:15.250Z,2,129025,10,255,8,3f,9f,…` | canboat `analyzer`, Signal K, log files |
| Signal K | a delta in JSON, SI units | Signal K servers and clients |

**Only** limits an output to some tags; leave it empty to send everything. The tags are
sentence types for NMEA 0183 (`RMC, GGA, HDT, VDM`), PGNs for NMEA 2000
(`129025, 129026, 130306`) and sensors for Signal K (`gps, wind, ais`, and `vessel` for the
own-ship identity). The editor lists the tags **Available** for the chosen format.

**Where outputs are kept.** There are two kinds, and the list tells them apart:

- Outputs that **come with a scenario** belong to it. Use **Scenario › Save as…** to keep
  changes to them.
- Outputs **you add** are kept on this computer, and marked *kept on this computer*. They
  start every time the app opens, whichever scenario you load, and your edits, switches and
  removals are remembered too. So the connection to your own software is set up once, not once
  per scenario.

Recordings are not kept: each one is a new file.

### NMEA 2000 and Signal K details

**NMEA 2000.** Each simulated device has its own source address on the bus: GPS 10, compass
11, speed log 12, depth 13, wind 14, water temperature 15, weather 16, rudder 17, attitude 18,
engine 19, AIS 20. Each one announces itself with an address claim (60928) and product
information (126996) when an NMEA 2000 output starts, and then every minute, so tools that list
the devices on the bus show them. Messages longer than 8 bytes (GNSS position, AIS) are sent as
fast packets: in Yacht Devices RAW they are split into CAN frames with a sequence counter, while
the other two formats carry the whole message on one line. The compass reports a magnetic
heading with its deviation and variation, as a real fluxgate compass does. Field layouts follow
the canboat PGN database.

**Signal K.** Own-ship data goes to the context `vessels.urn:mrn:imo:mmsi:<your MMSI>`. Every
30 seconds the simulator also sends the vessel's name, MMSI, call sign and dimensions. AIS
vessels get their own `vessels.` contexts, aids to navigation `atons.` and base stations
`shore.basestations.`. On a TCP server or WebSocket output, every new client first receives the
Signal K hello, which names the own vessel. A Signal K WebSocket output also answers HTTP
`GET /signalk` with the discovery document, which points at `ws://<host>/signalk/v1/stream`.
The stream is served on any path. The simulator does not implement the Signal K REST API, so
clients that need it must go through a Signal K server (see [section 13](#13-connecting-other-software)).

**Port already in use:** the output shows a *warning* when another program holds the port on
127.0.0.1 only (some clients may then reach that program instead), and an *error* when it
holds the whole port. Pick another port.

### Recording

Click **⏺ Record to file…** in Outputs (or **Data › Record…**) and choose a file. The dialog
opens in the output folder, with a name made of the date and time
(`marine-sim-20260621-143015.nmea`). This adds a file output named *Record* and the file name,
with TAG-block timestamps, so every line looks like:

```
\c:1782052215*5B\$GPRMC,143015.00,A,2824.0000,N,08033.0000,W,6.1,200.3,210626,6.5,W,A*hh
```

Untick or remove the output to stop recording. It is a ship's log that writes itself.

### Replay

Replay plays a log back to the outputs, at the pace it was recorded: one made here, or one
brought home from a real boat. Open the **Replay** section (or the **Data** tab) and click
**Open log…**; the dialog opens in the output folder. NMEA 0183 and AIS logs of up to 512 MB
are accepted (`.nmea`, `.log`, `.txt`, `.csv`). NMEA 2000 and Signal K logs cannot be replayed
yet.

| Control | Effect |
|---|---|
| **▶ Play** / **⏸ Pause** | Play or pause the log |
| **⏮** | Back to the start |
| Speed | Playback speed, from 0.5× to 60× |
| **Loop** | Start again at the end |
| Slider | Jump to any time in the log |
| **Mute simulated data while a log is loaded** | On by default: only the log goes to the outputs. Untick it to mix the log with simulated data. |
| **Close** | Unload the log and return to the simulation |

The section's title tells you where you stand: **Replay (loaded)** or **Replay (playing)**.
Under the slider are the position in the log, the time of the data and the line number.

The **pace** of the replay comes from the first source available in the log:

1. **TAG blocks** (`\c:…\`), in seconds or milliseconds.
2. Otherwise, **times inside the sentences**: RMC, GGA, GLL, ZDA, GNS…, handling the
   roll-over at midnight.
3. Otherwise, a fixed **10 lines per second**.

Lines between two timestamps are spread evenly. Replayed lines go to the NMEA 0183 outputs and
respect each output's **Only** filter. The magenta marker on the chart shows the last position
read from the log.

---

## 10. Preferences and saved settings

Open **Preferences** with **⚙** in the title bar, **View › Preferences…** or **⌘,** (Ctrl+, on
Windows and Linux).

| Preference | Effect |
|---|---|
| **Palette** | Day, Dusk or Night (also on the **View** tab). Until you pick one, the app follows the system's light or dark mode. |
| **Language** | English, Français or Deutsch. It changes at once; until you pick one, the app follows the system's language. Units (kn, nm, °) and the data sent are the same in every language. |
| **Output folder** | Where recordings and new file outputs are written, and where **Open log…** starts. **Choose…** picks another folder; **Default** goes back to your Documents folder. |
| **MCP server** | **Let agents connect** starts a server through which AI agents can watch and drive the simulation, on the **Port** below (10115 unless you change it). Off until you tick it. See [section 14](#14-ai-agents-mcp). |

Changes apply and are saved at once; **Done** closes the window. The bottom of the window
shows where the preferences file is kept.

The app keeps two files in your settings folder:

| File | Holds |
|---|---|
| `preferences.toml` | The preferences above |
| `outputs.toml` | The outputs you added ([section 9](#9-outputs-recording-and-replay)) |

| System | Settings folder |
|---|---|
| macOS | `~/Library/Application Support/marine-sim/` |
| Windows | `%APPDATA%\marine-sim\` |
| Linux | `~/.config/marine-sim/` |

Both are plain TOML files: you can copy them to another computer, or delete them to go back to
the defaults. Chart tiles are cached separately, in the system's cache folder.

Everything else (vessel, environment, sensors, AIS traffic, the scenario's outputs) belongs to
the scenario and is kept with **Scenario › Save as…**.

---

## 11. Command line (CLI)

The `cli` program runs the same simulation without a window, for servers, scripts and CI. No
chart, no ribbon, just the data.

```sh
cargo run --release -p cli -- [OPTIONS]
# or, after building: target/release/marine-sim-cli [OPTIONS]
```

| Option | Effect |
|---|---|
| `-s, --scenario FILE` | Load a scenario file (default: the built-in Cape Canaveral loop) |
| `-b, --builtin NAME` | Load a built-in scenario by file name (not together with `--scenario`) |
| `--list` | List the built-in scenarios and exit |
| `-p, --print` | Print every line to stdout |
| `--print-format FORMAT` | What `--print` shows: `nmea0183` (default), `yd_raw`, `actisense_ascii`, `canboat_plain` or `signalk` |
| `-d, --duration SECONDS` | Stop after this many real seconds |
| `--time-scale X` | Run time X times faster |
| `--record FILE` | Also record every line to FILE, with TAG blocks |
| `--replay FILE` | Replay a log to the outputs; simulated data is muted |
| `--replay-speed X` | Replay speed (default 1) |
| `--replay-loop` | Loop the log |
| `--exit-after-replay` | Exit when the log ends |
| `--mcp-port PORT` | Let AI agents connect at `http://127.0.0.1:PORT/mcp` ([section 14](#14-ai-agents-mcp)) |
| `--mcp-stdio` | Talk to an AI agent on stdin and stdout, for an agent that starts the program itself; runs until the agent closes them (not together with `--print`, `--duration` or `--mcp-port`) |
| `--write-default-scenario PATH` | Write the default scenario to PATH and exit, as a starting point for your own |

Log messages go to stderr; set `RUST_LOG=debug` for more detail.

**Examples**

```sh
# Ten minutes of simulated data in one minute, saved to a file
cargo run --release -p cli -- --time-scale 10 --duration 60 --record ten-minutes.nmea

# Serve a real log over TCP 10113, looping, at twice the speed
cargo run --release -p cli -- --replay passage.nmea --replay-speed 2 --replay-loop

# Count the sentence types of a 5-second run
cargo run --release -p cli -- --duration 5 --print | cut -c1-6 | sort | uniq -c
```

---

## 12. Scenario files

A scenario is a TOML file. Only `[initial]` is required; every other section has defaults.
Relative paths (polar, soundings CSV) are resolved from the scenario file's folder. The
easiest ways to start one: set things up in the app and use **Save as…**, run
`--write-default-scenario` for a commented example, or look in [`scenarios/`](../scenarios).

```toml
name = "My scenario"

[simulation]
time_scale = 1.0
paused = false
seed = 1                          # random seed: the same seed gives the same run
# start_time = "2026-06-21T14:00:00Z"   # omit to start at the current time

[vessel]
name = "SIMULATOR"
mmsi = 100000001
call_sign = "SIM01"
propulsion = "sail"               # "sail" or "motor"
# polar_file = "my-boat.pol"
polar_efficiency = 0.95
length_m = 12.0
beam_m = 4.0
draft_m = 2.0
max_speed_kn = 8.0
roll_period_s = 4.5

[vessel.ais]                      # own ship's transponder (!AIVDO)
enabled = true
class = "b"                       # "a" or "b"

[initial]
position = { lat = 28.40, lon = -80.55 }
heading = 200.0
speed = 6.0
target_speed = 6.0                # knots under engine
engine_on = false

[steering]
mode = "route"                    # "heading" (heading = …), "wind_angle" (twa = …) or "route"
looped = true
arrival_radius_m = 80.0
waypoints = [ { lat = 28.37, lon = -80.56 }, { lat = 28.30, lon = -80.54 } ]

[environment]                     # base values, keys as in section 5
wind_speed = 13.0
wind_direction = 110.0
current_speed = 0.3
current_set = 0.0
seabed_depth = 12.0

[dynamics]
pressure_trend_hpa_h = -0.5

[dynamics.wind]
gust_fraction = 0.12
gust_period_s = 20.0
shift_deg = 6.0
shift_period_s = 180.0

[tide]
enabled = true
range_m = 1.1
period_h = 12.42
# high_water = "2026-06-21T10:30:00Z"   # omit: high water at the start
flood_set_deg = 290.0
max_rate_kn = 0.3

[bathymetry]                      # optional
neighbours = 4
soundings = [ { lat = 28.38, lon = -80.55, depth = 11.0 } ]
# csv = "depths.csv"              # one "lat,lon,depth" per line

[overrides.environment]           # keys as in section 5
wind_speed = { mode = "offset", value = 5.0 }

[overrides.channels]              # keys as in section 6
depth = { mode = "replace", value = 2.0 }
heading_true = { mode = "freeze", value = 0.0 }

[[sensors]]                       # omit to get every sensor with defaults; once you list any,
                                  # only the listed ones run (plus AIS)
kind = "compass"                  # gps, compass, speed_log, depth_sounder, wind, water_temp,
rate_hz = 10.0                    # weather, rudder, attitude, engine, ais
talker = "HC"
angle_bias_deg = 1.5
deviation = { a = 0.5, b = 3.0, c = -1.0 }

[[outputs]]                       # omit [[outputs]] to get the default TCP + UDP on 10113
name = "OpenCPN"
type = "tcp_server"
bind = "0.0.0.0:10113"

[[outputs]]
name = "Plotter"
type = "serial"
port = "/dev/ttyUSB0"             # COM3 on Windows
baud = 4800
filter = ["RMC", "GGA", "HDT", "MWV", "DPT"]

# Other types:
#   type = "udp",        target = "127.0.0.1:10113", broadcast = false
#   type = "tcp_client", target = "192.168.1.10:10110"
#   type = "web_socket", bind = "0.0.0.0:10114"
#   type = "file",       path = "log.nmea", tag_blocks = true

[[outputs]]                       # NMEA 2000 as a Yacht Devices gateway would send it
name = "N2K"
type = "tcp_server"
bind = "0.0.0.0:1457"
format = "yd_raw"                 # nmea0183 (default), yd_raw, actisense_ascii, canboat_plain, signalk

[[outputs]]
name = "Signal K"
type = "web_socket"
bind = "0.0.0.0:3013"
format = "signalk"

[[ais.targets]]
mmsi = 100100002
name = "ATLANTIC TRADER"
call_sign = "WAT1002"
class = "a"                       # a, b, base_station, aton, sart
position = { lat = 28.20, lon = -80.43 }
nav_status = 0                    # 0 under way, 1 at anchor, 5 moored, 7 fishing…
motion = { mode = "course", cog = 355.0, sog_kn = 12.5 }
# motion = { mode = "route", sog_kn = 14.0, looped = false, waypoints = [ … ] }
# motion = { mode = "stationary" }

[ais.targets.static]
ship_type = 70                    # 30 fishing, 36 sailing, 60 passenger, 70 cargo, 80 tanker
imo = 9100002
to_bow = 140
to_stern = 40
to_port = 14
to_starboard = 14
draught_m = 9.2
destination = "JACKSONVILLE"
eta = "06-22 08:00"               # "MM-DD HH:MM" UTC
```

Aids to navigation also take `aton_type` (e.g. 29 = safe water), `virtual_aton` and
`off_position_m`. Any station can take `safety_text`.

Unknown keys are rejected with an error that names them, so a typo is caught at the dock and
not at sea.

---

## 13. Connecting other software

| Software | Connection |
|---|---|
| **OpenCPN** | Options → Connections → Add: Network, TCP, address `localhost`, port `10113`. Or UDP, port `10113`. |
| **Signal K server, NMEA 0183** | Add a data connection: NMEA 0183, TCP client, host `localhost`, port `10113`. |
| **Signal K server, NMEA 2000** | Add a TCP server output with data *NMEA 2000 · Yacht Devices RAW*, e.g. on port 1457. In Signal K, add a data connection: NMEA 2000, source *Yacht Devices YDWG-02 / YDEN-02 (TCP)*, host `localhost`, port 1457. |
| **Signal K client app** | Add a WebSocket server output with data *Signal K*, e.g. on `0.0.0.0:3013`, and point the client at `localhost:3013`. Clients that also need the REST API must connect through a Signal K server. |
| **canboat `analyzer`** | A TCP server output with data *NMEA 2000 · canboat plain*; `nc localhost <port> \| analyzer -json`. |
| **A hub that accepts data** | Add a TCP client output pointing at the hub's input port. |
| **A browser app** | Add a WebSocket server output, e.g. `0.0.0.0:10114`, and connect to `ws://localhost:10114`. |
| **A real instrument or plotter on the bench** | A USB-to-RS-422 adapter and a Serial port output at the instrument's baud rate. On the bench only: see [Safety](#safety-and-responsible-use). |
| **Another computer** | A TCP server on `0.0.0.0` (allow it through the firewall), or UDP broadcast to the LAN. |

A quick check from a terminal:

```sh
nc localhost 10113          # TCP: shows the stream
nc -ul 10113                # UDP: listen for datagrams (stop other listeners on 10113 first)
```

---

## 14. AI agents (MCP)

An AI agent such as Claude can watch and drive the simulator through the
[Model Context Protocol](https://modelcontextprotocol.io) (MCP). You can then ask in plain
words: "load the Gibraltar scenario, put a ship on a collision course from the starboard bow and
tell me what the AIS sentences say", or have an agent set up the conditions that the software
you are testing must handle.

### Switching it on

| You run | Do this | The agent connects to |
|---|---|---|
| The app | **Preferences › AI agents**: tick **Let agents connect** | `http://127.0.0.1:10115/mcp` (shown in Preferences, with a **Copy** button) |
| The CLI, already running | Start it with `--mcp-port 10115` | `http://127.0.0.1:10115/mcp` |
| Nothing: the agent starts the simulator | Give the agent the command `marine-sim-cli --mcp-stdio` | The program's stdin and stdout |

For example, with Claude Code:

```sh
claude mcp add --transport http marine-sim http://127.0.0.1:10115/mcp    # the running app
claude mcp add marine-sim -- /path/to/marine-sim-cli --mcp-stdio         # or headless
```

Other MCP clients take the same address, or the same command, in their own settings.

### What an agent can do

| Tools | What they do |
|---|---|
| `get_status`, `get_values` | Read the state: time, position, heading, speeds, depth, wind, steering, and every environment and instrument value with its override |
| `set_simulation` | Pause, resume, change the time scale |
| `steer`, `set_propulsion`, `move_vessel` | Hold a heading or a wind angle, follow a route, start the engine, set the speed, put the vessel somewhere else |
| `set_environment`, `set_override` | Change the wind, current, depth and the rest; replace, offset or freeze any value ([section 6](#6-overrides)) |
| `list_ais_targets`, `add_ais_target`, `add_collision_target`, `update_ais_target`, `remove_ais_targets` | Manage the AIS traffic, with range, bearing, CPA and TCPA of each target |
| `list_scenarios`, `load_scenario`, `get_scenario`, `reset_scenario` | Load a built-in scenario or one given as TOML text, read the current state as a scenario, reset |
| `list_sensors`, `set_sensor` | Switch an instrument off, change its rate, add errors |
| `list_outputs`, `add_output`, `remove_output` | See and manage the network outputs |
| `read_data` | Read the lines being sent, filtered by type or text: NMEA 0183 (with AIS), NMEA 2000 in any of its three text formats, or Signal K |

What an agent does shows in the app at once, as if you had done it yourself, and you can take
over at any moment. Each change is in force when the tool answers, so an agent can check its
result with the very next call.

`read_data` reads from the [traffic monitor](#traffic-monitor), which holds one format at a
time. When an agent asks for another format, the monitor switches to it, in the app too, and
starts afresh: the first answer holds only the lines sent since the switch.

### Limits, on purpose

- **This computer only.** The server listens on 127.0.0.1, and refuses requests that come from
  a web page. Anything that can run programs on your computer can connect while it is on, so it
  is off until you switch it on.
- **No files.** Through MCP the simulator neither reads nor writes files: a scenario that
  names a polar or soundings file is refused, and so are file and serial outputs. Use the app or
  the CLI for those.
- **Made-up identities.** New AIS targets get an MMSI that cannot belong to a real ship; an agent
  that sets a real-looking one gets a warning. The [safety rules](#safety-and-responsible-use)
  apply to agents as they do to you.
- Outputs an agent adds last until the next scenario is loaded; they are not kept in
  `outputs.toml`.
- Replay and recording are not available to agents.

---

## 15. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| An output shows an error "address in use" | Another program holds the port. Choose another port, or stop that program. |
| An output shows a warning about 127.0.0.1 | Another program listens on the same port, on 127.0.0.1 only. Local clients may reach it instead of the simulator. Use another port. |
| Nothing arrives over UDP | Check the address and port. For other computers, send to the broadcast address and tick **Broadcast**. Check the firewall. |
| A sentence type is missing | Is the sensor ticked? Does the output's **Only** filter include it? In the monitor, **»0** marks lines no output accepts. |
| Serial output: "lines dropped: … too slow for the traffic" | The traffic needs more bandwidth than the baud rate allows. Raise the baud rate, limit the output with **Only**, or lower the sensor rates. |
| A serial port is not listed | Plug in the adapter and reopen the **…** list in the output editor. On Linux, add your user to the `dialout` group. |
| An output I added is gone, or one is doubled | Outputs you add are kept in `outputs.toml` ([section 10](#10-preferences-and-saved-settings)). One that is also in the loaded scenario is started only once. |
| A recording or file output fails to open | Check the path and the **Output folder** in Preferences: the folder must exist and be writable. |
| **AGROUND** | The water is shallower than the draft. Right-click deeper water and pick **Move vessel here**, steer away, or override *Water depth (chart + tide)* in Environment. |
| The boat does not reach an upwind waypoint | She is tacking on the laylines, as a sailing boat must. Wait, or check the **Tack** row in Navigation. In a hurry, switch **Under** to **⚙ Engine**. |
| The boat stopped at the end of her route | The route is complete and **Loop** is off. Click a waypoint to resume, or turn **Loop** on. |
| Clicking the chart adds waypoints instead of selecting targets | **Add waypoints** is on. Turn it off on the **Chart** tab. |
| An NMEA 2000 tool lists no devices | Address claims go out when the output starts and then every minute; wait up to a minute after connecting. |
| A Signal K client shows no AIS targets | Many clients only subscribe to their own vessel by default. Choose *all* vessels in the client. |
| Replay runs too fast or too slow | The log has no TAG blocks or sentence times, so it plays at 10 lines per second. Set the speed, or record with TAG blocks. |
| Nothing but the log is sent during a replay | **Mute simulated data while a log is loaded** is on. Untick it, or **Close** the log. |
| Chart tiles do not load | The chart needs internet access. Tiles already loaded are cached and still work offline. |
| The scenario will not load | The error names the line and key. Unknown keys and wrong types are rejected. |
| The monitor changed format by itself | An AI agent read the data in another format ([section 14](#14-ai-agents-mcp)). Choose the format again at the left of the monitor. |
| An AI agent cannot connect | Is **Let agents connect** ticked in Preferences, and is the address the one shown there? "Could not start" under it means another program holds the port: change the **Port**. The agent must run on this computer. |

Fair winds, and may all your groundings be simulated.

---

Marine Data Simulator is free software under the GNU General Public License version 3
([LICENSE](../LICENSE)); third-party code and data are listed in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md). Map data © OpenStreetMap contributors,
© OpenSeaMap contributors.
