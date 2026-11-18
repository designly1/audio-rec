#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bluetooth;
mod monitor;

use bluetooth::{Command, Device, Event};
use eframe::egui;
use monitor::{Monitor, Quality};
use std::{
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};

const GREEN: egui::Color32 = egui::Color32::from_rgb(110, 225, 170);
const AMBER: egui::Color32 = egui::Color32::from_rgb(240, 200, 100);
const RED: egui::Color32 = egui::Color32::from_rgb(255, 150, 135);

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

struct ReceiverApp {
    commands: Sender<Command>,
    events: Receiver<Event>,
    devices: Vec<Device>,
    selected: Option<String>,
    status: String,
    busy: bool,
    connected: bool,
    enabled: bool,
    failed: bool,
    monitor: Option<Monitor>,
    connected_since: Option<Instant>,
    ever_connected: bool,
    link_drops: u32,
    reconnects: u32,
}

impl ReceiverApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (commands, events) = bluetooth::start();
        let mut app = Self {
            commands,
            events,
            devices: Vec::new(),
            selected: None,
            status: "Looking for paired audio devices…".into(),
            busy: false,
            connected: false,
            enabled: false,
            failed: false,
            monitor: None,
            connected_since: None,
            ever_connected: false,
            link_drops: 0,
            reconnects: 0,
        };
        app.send(Command::Refresh);
        app
    }

    fn send(&mut self, command: Command) {
        self.failed = false;
        if self.commands.send(command).is_ok() {
            self.busy = true;
        } else {
            self.status = "The Bluetooth service stopped. Please restart the app.".into();
            self.failed = true;
            self.busy = false;
        }
    }

    fn quality_panel(&self, ui: &mut egui::Ui) {
        let stats = self
            .monitor
            .as_ref()
            .map(Monitor::stats)
            .unwrap_or_default();
        let now = Instant::now();
        let per_min = stats.dropouts_since(now, Duration::from_secs(60));
        let playing = stats
            .last_audio
            .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(5));
        let quality = monitor::rating(self.connected, per_min, playing);
        let color = match quality {
            Quality::Good => GREEN,
            Quality::Fair => AMBER,
            Quality::Poor => RED,
            Quality::Offline | Quality::Measuring => ui.visuals().weak_text_color(),
        };
        ui.horizontal(|ui| {
            ui.strong("CONNECTION QUALITY");
            ui.colored_label(color, format!("● {}", quality.label()));
        });
        ui.add_space(6.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("Level");
                let fill = ((stats.level_db + 60.0) / 60.0).clamp(0.0, 1.0);
                let text = if stats.level_db <= -60.0 {
                    "silent".to_owned()
                } else {
                    format!("{:.0} dB", stats.level_db)
                };
                ui.add(
                    egui::ProgressBar::new(fill)
                        .desired_width((ui.available_width() - 70.0).max(60.0))
                        .fill(color),
                );
                ui.monospace(text);
            });
            ui.add_space(4.0);
            egui::Grid::new("quality")
                .num_columns(2)
                .spacing([24.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Uptime");
                    ui.label(
                        self.connected_since
                            .map(|t| format_duration(now.saturating_duration_since(t)))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.end_row();
                    ui.label("Audio dropouts");
                    ui.label(format!(
                        "{per_min} in last minute · {} this session",
                        stats.dropouts.len()
                    ));
                    ui.end_row();
                    ui.label("Last dropout");
                    ui.label(
                        stats
                            .dropouts
                            .last()
                            .map(|t| {
                                format!("{} ago", format_duration(now.saturating_duration_since(*t)))
                            })
                            .unwrap_or_else(|| "none".into()),
                    );
                    ui.end_row();
                    ui.label("Link drops");
                    ui.label(format!("{} · {} reconnects", self.link_drops, self.reconnects));
                    ui.end_row();
                });
            if let Some(error) = &stats.error {
                ui.colored_label(RED, error);
            }
            ui.weak(
                "Dropouts are estimated from audio on your default output; other apps' sounds can affect it.",
            );
        });
    }
}

impl eframe::App for ReceiverApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Devices(devices) => {
                    self.devices = devices;
                    if !self
                        .devices
                        .iter()
                        .any(|d| Some(&d.id) == self.selected.as_ref())
                    {
                        self.selected = self.devices.first().map(|d| d.id.clone());
                    }
                    if !self.enabled {
                        self.status = if self.devices.is_empty() {
                            "No audio sources found. Pair your phone in Windows Bluetooth settings, then refresh.".into()
                        } else {
                            "Ready. Select your phone and click Connect.".into()
                        };
                    }
                }
                Event::Status { message, connected } => {
                    if connected && !self.connected {
                        self.connected_since = Some(Instant::now());
                        if self.ever_connected {
                            self.reconnects += 1;
                        }
                        self.ever_connected = true;
                    } else if !connected && self.connected && self.enabled {
                        self.link_drops += 1;
                        self.connected_since = None;
                    }
                    self.status = message;
                    self.connected = connected;
                    self.failed = false;
                }
                Event::Done => self.busy = false,
                Event::Released => {
                    self.enabled = false;
                    self.connected = false;
                    self.connected_since = None;
                    self.monitor = None;
                }
                Event::Error(error) => {
                    self.status = error;
                    self.failed = true;
                    self.busy = false;
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(18.0);
            ui.heading(egui::RichText::new("Bluetooth Receiver").size(28.0));
            ui.label("Your phone’s audio. Your PC’s speakers.");
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.strong("PAIRED AUDIO SOURCES");
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh"))
                    .clicked()
                {
                    self.send(Command::Refresh);
                }
            });
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(145.0)
                    .show(ui, |ui| {
                        if self.devices.is_empty() {
                            ui.weak("Your paired phone will appear here.");
                        }
                        for device in &self.devices {
                            ui.add_enabled_ui(!self.busy && !self.enabled, |ui| {
                                ui.selectable_value(
                                    &mut self.selected,
                                    Some(device.id.clone()),
                                    &device.name,
                                );
                            });
                        }
                    });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let device = self
                    .devices
                    .iter()
                    .find(|d| Some(&d.id) == self.selected.as_ref())
                    .cloned();
                if ui
                    .add_enabled(
                        !self.busy && !self.enabled && device.is_some(),
                        egui::Button::new("Connect").min_size(egui::vec2(110.0, 34.0)),
                    )
                    .clicked()
                    && let Some(device) = device
                {
                    self.status = format!("Connecting to {}…", device.name);
                    self.enabled = true;
                    self.connected_since = None;
                    self.ever_connected = false;
                    self.link_drops = 0;
                    self.reconnects = 0;
                    self.monitor = Some(Monitor::start());
                    self.send(Command::Connect(device));
                }
                if ui
                    .add_enabled(
                        !self.busy && self.enabled,
                        egui::Button::new("Disconnect").min_size(egui::vec2(110.0, 34.0)),
                    )
                    .clicked()
                {
                    self.send(Command::Disconnect);
                }
                if self.busy {
                    ui.spinner();
                }
            });
            ui.add_space(16.0);
            let color = if self.failed {
                RED
            } else if self.connected {
                GREEN
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, &self.status);
            if self.enabled {
                ui.add_space(14.0);
                self.quality_panel(ui);
            }
            ui.add_space(18.0);
            ui.separator();
            ui.label("1. Pair your phone with this PC in Windows Bluetooth settings.");
            ui.label("2. Refresh, select your phone, and connect.");
            ui.label("3. Play music on your phone. Keep this app open.");
            ui.add_space(8.0);
            ui.weak("Audio uses your Windows default output. Control volume on your phone or PC.");
            if ui.button("Open Bluetooth settings").clicked()
                && let Err(error) = std::process::Command::new("explorer.exe")
                    .arg("ms-settings:bluetooth")
                    .spawn()
            {
                self.status = format!("Could not open settings: {error}");
                self.failed = true;
            }
        });
    }
}

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "--diagnose") {
        let report =
            bluetooth::diagnose().unwrap_or_else(|error| format!("Diagnostic failed: {error}\n"));
        if let Some(path) = args.get(2) {
            if let Err(error) = std::fs::write(path, &report) {
                eprintln!("Cannot write diagnostics: {error}");
                std::process::exit(1);
            }
        } else {
            println!("{report}");
        }
        if report.starts_with("Diagnostic failed:") {
            std::process::exit(1);
        }
        return Ok(());
    }
    eframe::run_native(
        "Bluetooth Receiver",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([540.0, 660.0])
                .with_min_inner_size([480.0, 600.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ReceiverApp::new(cc)))),
    )
}
