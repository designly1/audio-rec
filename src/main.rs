#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bluetooth;

use bluetooth::{Command, Device, Event};
use eframe::egui;
use std::{
    sync::mpsc::{Receiver, Sender},
    time::Duration,
};

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
                    self.status = message;
                    self.connected = connected;
                    self.failed = false;
                }
                Event::Done => self.busy = false,
                Event::Released => {
                    self.enabled = false;
                    self.connected = false;
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
                egui::Color32::from_rgb(255, 150, 135)
            } else if self.connected {
                egui::Color32::from_rgb(110, 225, 170)
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, &self.status);
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
                .with_inner_size([540.0, 500.0])
                .with_min_inner_size([480.0, 460.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ReceiverApp::new(cc)))),
    )
}
