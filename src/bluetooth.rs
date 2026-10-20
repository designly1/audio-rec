use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use windows::{
    Devices::Enumeration::DeviceInformation,
    Media::Audio::{
        AudioPlaybackConnection, AudioPlaybackConnectionOpenResultStatus as OpenStatus,
        AudioPlaybackConnectionState,
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::{HSTRING, Result},
};

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
}

pub enum Command {
    Refresh,
    Connect(Device),
    Disconnect,
}

pub enum Event {
    Devices(Vec<Device>),
    Status { message: String, connected: bool },
    Done,
    Released,
    Error(String),
}

// Every WinRT object stays on this initialized worker thread. Drop objects before
// uninitializing its apartment, including on errors and normal shutdown.
struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        // SAFETY: this fresh worker thread has no existing COM apartment.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful initialization on this same thread.
        unsafe { RoUninitialize() };
    }
}

struct Connection(AudioPlaybackConnection);
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}

fn devices() -> Result<Vec<Device>> {
    let selector = AudioPlaybackConnection::GetDeviceSelector()?;
    let found = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
    let mut devices = Vec::new();
    for device in found {
        devices.push(Device {
            id: device.Id()?.to_string(),
            name: device.Name()?.to_string(),
        });
    }
    devices.sort_by_key(|device| device.name.to_lowercase());
    Ok(devices)
}

fn open_failure(status: OpenStatus) -> Option<&'static str> {
    match status {
        OpenStatus::Success => None,
        OpenStatus::RequestTimedOut => {
            Some("The phone did not respond. Wake it, check Bluetooth, and try again.")
        }
        OpenStatus::DeniedBySystem => Some(
            "Windows denied the connection. Check Bluetooth settings and close other receiver apps.",
        ),
        _ => Some(
            "Windows could not open the audio connection. Reconnect the phone in Bluetooth settings and retry.",
        ),
    }
}

fn connect(device: &Device) -> std::result::Result<Connection, String> {
    let connection = Connection(
        AudioPlaybackConnection::TryCreateFromId(&HSTRING::from(&device.id))
            .map_err(|e| format!("This device cannot receive an audio connection: {e}"))?,
    );
    connection
        .0
        .StartAsync()
        .and_then(|action| action.join())
        .map_err(|e| e.to_string())?;
    let result = connection
        .0
        .OpenAsync()
        .and_then(|operation| operation.join())
        .map_err(|e| e.to_string())?;
    let status = result.Status().map_err(|e| e.to_string())?;
    if let Some(message) = open_failure(status) {
        let detail = result
            .ExtendedError()
            .map(|code| format!(" (0x{:08X})", code.0 as u32))
            .unwrap_or_default();
        return Err(format!("{message}{detail}"));
    }
    Ok(connection)
}

pub fn start() -> (Sender<Command>, Receiver<Event>) {
    let (commands, input) = mpsc::channel();
    let (output, events) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(error) = run(input, &output) {
            let _ = output.send(Event::Error(error.to_string()));
        }
    });
    (commands, events)
}

fn run(input: Receiver<Command>, output: &Sender<Event>) -> Result<()> {
    let _apartment = Apartment::new()?;
    let mut active: Option<(Device, Connection)> = None;
    let mut last_state = None;
    loop {
        match input.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::Refresh) => {
                let event = match devices() {
                    Ok(found) => Event::Devices(found),
                    Err(error) => Event::Error(format!("Cannot find audio devices: {error}")),
                };
                let _ = output.send(event);
                let _ = output.send(Event::Done);
            }
            Ok(Command::Connect(device)) => {
                // Close the previous sink before opening a new one.
                active.take();
                last_state = None;
                match connect(&device) {
                    Ok(connection) => active = Some((device, connection)),
                    Err(error) => {
                        let _ = output.send(Event::Released);
                        let _ = output.send(Event::Error(error));
                    }
                }
                let _ = output.send(Event::Done);
            }
            Ok(Command::Disconnect) => {
                // Surface close errors instead of claiming disconnection succeeded.
                if let Some((_, connection)) = active.as_ref()
                    && let Err(error) = connection.0.Close()
                {
                    let _ = output.send(Event::Error(format!("Could not disconnect: {error}")));
                    let _ = output.send(Event::Done);
                    continue;
                }
                active.take();
                last_state = None;
                let _ = output.send(Event::Released);
                let _ = output.send(Event::Status {
                    message: "Disconnected. Choose a device to listen again.".into(),
                    connected: false,
                });
                let _ = output.send(Event::Done);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some((device, connection)) = active.as_ref() {
            match connection.0.State() {
                Ok(state) if last_state != Some(state) => {
                    last_state = Some(state);
                    let opened = state == AudioPlaybackConnectionState::Opened;
                    let message = if opened {
                        format!("Connected to {}. Play audio on your phone.", device.name)
                    } else {
                        format!(
                            "Waiting for {}. If disconnected, click Disconnect and reconnect.",
                            device.name
                        )
                    };
                    let _ = output.send(Event::Status {
                        message,
                        connected: opened,
                    });
                }
                Err(error) => {
                    let _ = output.send(Event::Released);
                    let _ = output.send(Event::Error(format!("Connection lost: {error}")));
                    active.take();
                    last_state = None;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub fn diagnose() -> std::result::Result<String, String> {
    std::thread::spawn(|| {
        let _apartment = Apartment::new().map_err(|e| e.to_string())?;
        let devices = devices().map_err(|e| e.to_string())?;
        let mut report = format!(
            "Windows Bluetooth audio API: available\nEligible devices: {}\n",
            devices.len()
        );
        for device in devices {
            report.push_str(&format!("- {}\n", device.name));
        }
        Ok(report)
    })
    .join()
    .map_err(|_| "Diagnostic worker stopped unexpectedly".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_success_is_accepted() {
        assert!(open_failure(OpenStatus::Success).is_none());
        for status in [
            OpenStatus::RequestTimedOut,
            OpenStatus::DeniedBySystem,
            OpenStatus::UnknownFailure,
            OpenStatus(999),
        ] {
            assert!(open_failure(status).is_some());
        }
    }
}
