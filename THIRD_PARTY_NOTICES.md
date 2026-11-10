# Third-party notices

Marine Data Simulator is licensed under the GNU General Public License version 3
([LICENSE](LICENSE)). It includes or is built from the works below, under their own licenses,
all compatible with the GPL-3.0.

## Adapted code

### SolidCraft

`crates/app/src/ui/title_bar.rs` (the title bar under the macOS traffic lights: hidden OS
title bar, drag area, double-click to zoom) follows SolidCraft's
`crates/ui-egui/src/window_chrome.rs` (https://github.com/tangency-labs/solidcraft), licensed
MIT OR Apache-2.0. Used here under the MIT License:

```text
MIT License

Copyright (c) 2026 SolidCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rust crates

The app is built from Rust crates listed in `Cargo.lock`, among them egui and eframe (UI),
walkers (map), tokio (networking), serialport, rfd (file dialogs), serde and toml. Their
licenses are permissive: MIT, Apache-2.0 (some with the LLVM exception), BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, Unicode-3.0, BSL-1.0, MPL-2.0, CDLA-Permissive-2.0, 0BSD, CC0-1.0 or
Unlicense; where a crate offers a choice, a permissive one is used. Each crate's license text
and copyright notice ship with its source (`cargo vendor` or crates.io).

To list them: `cargo metadata --format-version 1` (field `license` of each package).

## Fonts

egui bundles fonts for text and symbols (crate `epaint_default_fonts`, which carries their
license texts): Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT; its Bitstream Vera parts
under the Bitstream Vera License), Noto Emoji (SIL Open Font License 1.1) and emoji-icon-font
(MIT).

## Map data

Map tiles are loaded at run time and not distributed with the app.
© OpenStreetMap contributors (https://www.openstreetmap.org/copyright, ODbL 1.0);
© OpenSeaMap contributors (https://www.openseamap.org, CC BY-SA 2.0).
