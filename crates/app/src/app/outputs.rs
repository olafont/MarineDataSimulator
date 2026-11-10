//! Outputs section of the right panel: the list of outputs and the output editor.

use egui::{CollapsingHeader, ComboBox, RichText, TextEdit, Ui};
use engine::sim_sensors::SensorKind;
use engine::transports::{DataFormat, OutputConfig, OutputKind, Status};

use super::SimApp;
use crate::ui::form::{Width, form};
use crate::ui::theme::{Tone, tokens};
use crate::t;
use crate::ui::i18n::tr;

impl SimApp {
    pub(super) fn outputs(&mut self, ui: &mut Ui) {
        let reveal = std::mem::take(&mut self.reveal_outputs);
        let header = CollapsingHeader::new(RichText::new(t!("Outputs")).strong()).id_salt("outputs").default_open(true);
        header.open(reveal.then_some(true)).show(ui, |ui| {
            let t = tokens(ui);
            for o in self.engine.outputs().list() {
                ui.horizontal(|ui| {
                    let mut enabled = o.config.enabled;
                    if ui.checkbox(&mut enabled, RichText::new(&o.config.name).strong()).changed() {
                        let mut cfg = o.config.clone();
                        cfg.enabled = enabled;
                        self.update_output(o.id, cfg);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text(t!("Remove")).clicked() {
                            self.remove_output(o.id);
                        }
                        if ui.small_button("✏").on_hover_text(t!("Edit")).clicked() {
                            let folder = self.output_folder();
                            self.output_editor = Some(OutputDraft::from_config(Some(o.id), &folder, &o.config));
                        }
                    });
                });
                ui.indent(("out", o.id), |ui| {
                    let (tone, text) = status_badge(&o.status);
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(t.tone(ui, tone), text);
                        ui.weak(format!("{} · {}", describe(&o.config.kind), o.config.format.short()));
                    });
                    let mut line = crate::tf!("{n} sent · {kb} kB", n = o.sent, kb = format!("{:.1}", o.bytes as f64 / 1024.0));
                    match o.config.kind {
                        OutputKind::TcpServer { .. } | OutputKind::WebSocket { .. } => {
                            line += &crate::tf!(" · {n} client(s)", n = o.clients);
                        }
                        OutputKind::TcpClient { .. } | OutputKind::Serial { .. } if o.clients == 0 => {
                            line += t!(" · not connected");
                        }
                        _ => {}
                    }
                    if o.errors > 0 {
                        line += &crate::tf!(" · {n} errors", n = o.errors);
                    }
                    if !o.config.filter.is_empty() {
                        line += &crate::tf!(" · only {tags}", tags = o.config.filter.join(","));
                    }
                    if self.local_outputs.contains(o.id) {
                        line += t!(" · kept on this computer");
                    }
                    let tip = if self.local_outputs.contains(o.id) {
                        t!("You added this output: it is kept on this computer and starts with every scenario.")
                    } else {
                        t!("From the scenario: saved with Scenario › Save as.")
                    };
                    ui.weak(line).on_hover_text(tip);
                });
            }
            ui.horizontal(|ui| {
                if ui.button(t!("+ Add output")).clicked() {
                    self.new_output();
                }
                let tip = t!("Adds a file output with TAG-block timestamps, for replay");
                if ui.button(t!("⏺ Record to file…")).on_hover_text(tip).clicked() {
                    self.record_to_file();
                }
            });
            self.output_editor_ui(ui);
        });
    }

    fn output_editor_ui(&mut self, ui: &mut Ui) {
        let folder = self.output_folder();
        let Some(d) = &mut self.output_editor else {
            return;
        };
        let mut close = false;
        let mut apply = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong(if d.id.is_some() { t!("Edit output") } else { t!("New output") });
            form(ui, "out_edit", |f| {
                let t = f.tokens();
                f.text(t!("Name"), &mut d.name, Width::Long, 0);
                let kinds: Vec<(OutKind, &str)> = OutKind::ALL.iter().map(|&k| (k, k.label())).collect();
                if f.choice(t!("Type"), &mut d.kind, &kinds) {
                    d.addr = d.kind.default_addr().into();
                }
                let formats: Vec<(DataFormat, String)> = DataFormat::ALL.iter().map(|&k| (k, tr(k.label()))).collect();
                f.choice(t!("Data"), &mut d.format, &formats);
                match d.kind {
                    OutKind::Udp | OutKind::TcpServer | OutKind::TcpClient | OutKind::WebSocket => {
                        let label = match d.kind {
                            OutKind::Udp | OutKind::TcpClient => t!("Send to"),
                            _ => t!("Listen on"),
                        };
                        f.text(label, &mut d.addr, Width::Medium, 0);
                        if d.kind == OutKind::Udp {
                            f.check("", &mut d.broadcast, t!("Broadcast"));
                        }
                    }
                    OutKind::Serial => {
                        f.row(t!("Port"), |ui| {
                            ui.add(
                                TextEdit::singleline(&mut d.port)
                                    .hint_text(t!("/dev/tty.usbserial or COM3"))
                                    .desired_width(t.field_medium),
                            );
                            ComboBox::from_id_salt("ports").selected_text("…").width(t.space_xl * 2.0).show_ui(
                                ui,
                                |ui| {
                                    let ports = engine::transports::available_ports();
                                    if ports.is_empty() {
                                        ui.weak(t!("no ports found"));
                                    }
                                    for p in ports {
                                        if ui.selectable_label(d.port == p, &p).clicked() {
                                            d.port = p;
                                        }
                                    }
                                },
                            );
                        });
                        let bauds: Vec<(u32, String)> = [4800u32, 9600, 19_200, 38_400, 57_600, 115_200]
                            .iter()
                            .map(|&b| (b, b.to_string()))
                            .collect();
                        f.choice(t!("Baud"), &mut d.baud, &bauds);
                    }
                    OutKind::File => {
                        f.row(t!("File"), |ui| {
                            ui.add(TextEdit::singleline(&mut d.path).desired_width(t.field_medium));
                            if ui.small_button("…").clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .add_filter(t!("NMEA log"), &["nmea", "log", "txt"])
                                    .set_directory(&folder)
                                    .save_file()
                            {
                                d.path = p.display().to_string();
                            }
                        });
                        if d.format == DataFormat::Nmea0183 {
                            f.check("", &mut d.tag_blocks, t!("TAG-block timestamps (for replay)"));
                        } else {
                            f.readout("", t!("Timestamps are part of this format."));
                        }
                    }
                }
                f.text_hint(t!("Only"), &mut d.filter, Width::Long, 0, &crate::tf!("all, or {tags}", tags = d.format.filter_hint()));
            });
            ui.weak(crate::tf!("Available: {tags}", tags = available_tags(d.format)));
            if d.format == DataFormat::SignalK && d.kind == OutKind::WebSocket {
                ui.weak(t!("Clients receive a Signal K hello, and GET /signalk returns the discovery document."));
            }
            ui.horizontal(|ui| {
                if ui.button(t!("Apply")).clicked() {
                    apply = Some(d.to_config());
                }
                if ui.button(t!("Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if let Some(cfg) = apply {
            match d.id {
                Some(id) => self.update_output(id, cfg),
                None => self.add_local_output(cfg),
            }
            close = true;
        }
        if close {
            self.output_editor = None;
        }
    }
}

/// Where an output sends, in the interface's language.
fn describe(k: &OutputKind) -> String {
    match k {
        OutputKind::Udp { target, broadcast: true } => crate::tf!("UDP broadcast to {to}", to = target),
        OutputKind::Udp { target, .. } => crate::tf!("UDP to {to}", to = target),
        OutputKind::TcpServer { bind } => crate::tf!("TCP server {addr}", addr = bind),
        OutputKind::TcpClient { target } => crate::tf!("TCP client to {to}", to = target),
        OutputKind::WebSocket { bind } => format!("WebSocket ws://{bind}"),
        OutputKind::Serial { port, baud } => crate::tf!("Serial {port} @ {baud}", port = port, baud = baud),
        OutputKind::File { path, .. } => crate::tf!("File {path}", path = path),
    }
}

/// The colour and text that tell an output's state.
pub(super) fn status_badge(s: &Status) -> (Tone, String) {
    match s {
        Status::Running => (Tone::Ok, t!("⚫ running").into()),
        Status::Starting => (Tone::Warn, t!("⚫ starting").into()),
        Status::Warning(w) => (Tone::Warn, format!("⚠ {w}")),
        Status::Disabled => (Tone::Muted, t!("○ off").into()),
        Status::Failed(e) => (Tone::Error, format!("⚫ {e}")),
    }
}

/// The tags an output filter can name in a format.
fn available_tags(format: DataFormat) -> String {
    match format {
        DataFormat::Nmea0183 => {
            let mut v = protocols::nmea0183::SENTENCES.to_vec();
            v.extend(["VDM", "VDO"]);
            v.join(" ")
        }
        DataFormat::SignalK => {
            let mut v: Vec<&str> = SensorKind::ALL.iter().map(|&k| protocols::signalk::sensor_key(k)).collect();
            v.push("vessel");
            v.join(" ")
        }
        _ => protocols::n2k::PGNS.iter().map(u32::to_string).collect::<Vec<_>>().join(" "),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutKind {
    Udp,
    TcpServer,
    TcpClient,
    WebSocket,
    Serial,
    File,
}

impl OutKind {
    const ALL: &'static [OutKind] =
        &[OutKind::Udp, OutKind::TcpServer, OutKind::TcpClient, OutKind::WebSocket, OutKind::Serial, OutKind::File];

    fn label(self) -> &'static str {
        match self {
            OutKind::Udp => "UDP",
            OutKind::TcpServer => t!("TCP server"),
            OutKind::TcpClient => t!("TCP client"),
            OutKind::WebSocket => t!("WebSocket server"),
            OutKind::Serial => t!("Serial port"),
            OutKind::File => t!("File (record)"),
        }
    }

    fn default_addr(self) -> &'static str {
        match self {
            OutKind::Udp => "127.0.0.1:10113",
            OutKind::TcpServer => "0.0.0.0:10113",
            OutKind::TcpClient => "127.0.0.1:10110",
            OutKind::WebSocket => "0.0.0.0:10114",
            OutKind::Serial | OutKind::File => "",
        }
    }
}

pub(super) struct OutputDraft {
    id: Option<u64>,
    name: String,
    enabled: bool,
    kind: OutKind,
    addr: String,
    broadcast: bool,
    port: String,
    baud: u32,
    path: String,
    tag_blocks: bool,
    format: DataFormat,
    filter: String,
}

impl OutputDraft {
    /// A draft of `c`; a file output it becomes is written in `folder` unless the user picks
    /// another file.
    pub(super) fn from_config(id: Option<u64>, folder: &std::path::Path, c: &OutputConfig) -> Self {
        let mut d = Self {
            id,
            name: c.name.clone(),
            enabled: c.enabled,
            kind: OutKind::Udp,
            addr: String::new(),
            broadcast: false,
            port: String::new(),
            baud: 4800,
            path: folder.join("marine-sim.nmea").display().to_string(),
            tag_blocks: true,
            format: c.format,
            filter: c.filter.join(", "),
        };
        match &c.kind {
            OutputKind::Udp { target, broadcast } => (d.addr, d.broadcast) = (target.clone(), *broadcast),
            OutputKind::TcpServer { bind } => (d.kind, d.addr) = (OutKind::TcpServer, bind.clone()),
            OutputKind::TcpClient { target } => (d.kind, d.addr) = (OutKind::TcpClient, target.clone()),
            OutputKind::WebSocket { bind } => (d.kind, d.addr) = (OutKind::WebSocket, bind.clone()),
            OutputKind::Serial { port, baud } => (d.kind, d.port, d.baud) = (OutKind::Serial, port.clone(), *baud),
            OutputKind::File { path, tag_blocks } => (d.kind, d.path, d.tag_blocks) = (OutKind::File, path.clone(), *tag_blocks),
        }
        d
    }

    fn to_config(&self) -> OutputConfig {
        let addr = self.addr.trim().to_string();
        OutputConfig {
            name: self.name.trim().to_string(),
            enabled: self.enabled,
            kind: match self.kind {
                OutKind::Udp => OutputKind::Udp { target: addr, broadcast: self.broadcast },
                OutKind::TcpServer => OutputKind::TcpServer { bind: addr },
                OutKind::TcpClient => OutputKind::TcpClient { target: addr },
                OutKind::WebSocket => OutputKind::WebSocket { bind: addr },
                OutKind::Serial => OutputKind::Serial { port: self.port.trim().to_string(), baud: self.baud },
                OutKind::File => OutputKind::File { path: self.path.trim().to_string(), tag_blocks: self.tag_blocks },
            },
            format: self.format,
            filter: self
                .filter
                .split([',', ' '])
                .map(|s| s.trim().to_ascii_uppercase())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}
