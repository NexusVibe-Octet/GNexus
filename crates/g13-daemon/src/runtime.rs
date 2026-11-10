//! Everything `g13d` runs, packaged so that the GUI can embed it in its own
//! process: HID thread + engine thread (with the profile manager), plus a
//! [`UiEvent`] stream and a small request channel.
//!
//! ```text
//!  Runtime::start ─┬─ thread "g13-hid"    (HidWorker: sole owner of the device)
//!                  ├─ thread "g13-engine" (Engine::run_with + ProfileManager::tick)
//!                  │     ▲ Request (rescan…)        │ UiEvent ──▶ caller
//!                  ├─ thread "mouse"      (MouseWorker: sole owner of the mouse;
//!                  │     MouseCommand in, MouseStatus out as UiEvent::Mouse via the engine)
//!                  └─ thread "headset"    (HeadsetWorker: sole owner of the headset dongle;
//!                        HeadsetCommand in, HeadsetStatus out as UiEvent::Headset)
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use crate::device::hid::{HidConfig, HidWorker, ReadMode};
use crate::engine::{Engine, UiEvent};
use crate::lcd::LcdManager;
use crate::headset::{HeadsetCommand, HeadsetWorker};
use crate::mouse::{MouseCommand, MouseWorker};
use crate::platform::{self, InputInjector, mock::RecordingInjector};
use crate::profiles::{ProfileManager, ProfileStore, store};

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    /// Log what would be injected instead of sending it.
    pub dry_run: bool,
    /// `GetInputReport` polling instead of interrupt reads.
    pub poll: bool,
    /// Profile directory (`None` = standard config location).
    pub profiles_dir: Option<PathBuf>,
    /// Switch profiles automatically on focus change.
    pub auto_switch: bool,
    /// G-key that opens the GUI overlay instead of its binding.
    pub overlay_key: Option<g13_core::G13Key>,
    /// Also drive a Logitech HID++ mouse (G502 X PLUS…).
    pub mouse: bool,
    /// Also drive a Logitech HID++ headset (PRO X Wireless…).
    pub headset: bool,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self { dry_run: false, poll: false, profiles_dir: None, auto_switch: true, overlay_key: None, mouse: true, headset: true }
    }
}

/// Requests executed on the engine thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Re-read the profile directory now (after the GUI saved a file)
    /// instead of waiting for the next periodic scan.
    RescanProfiles,
    /// Profile edited in the GUI: active on the G13 while the GUI window
    /// has the focus (`None` = back to normal auto-switching).
    Preview(Option<String>),
    /// Change the G-key reserved for the overlay.
    SetOverlayKey(Option<g13_core::G13Key>),
    /// "Test everywhere": force this profile in every application
    /// (`None` = automatic switching again).
    Pin(Option<String>),
    /// Forward an event to the subscribers (from the mouse thread).
    Emit(UiEvent),
}

pub struct Runtime {
    shutdown: Arc<AtomicBool>,
    recent: crate::profiles::manager::RecentApps,
    requests: Sender<Request>,
    profiles_dir: Option<PathBuf>,
    lcd_lang: crate::lcd::i18n::LangCell,
    engine: Option<JoinHandle<()>>,
    hid: Option<JoinHandle<()>>,
    mouse: Option<(Sender<MouseCommand>, JoinHandle<()>)>,
    headset: Option<(Sender<HeadsetCommand>, JoinHandle<()>)>,
}

impl Runtime {
    /// Start the HID and engine threads. Returns the runtime and the stream
    /// of [`UiEvent`]s (the first one is the initial `ActiveProfile`).
    pub fn start(opts: RuntimeOptions) -> Result<(Self, Receiver<UiEvent>), String> {
        // keys must not wait behind a game that saturates the CPU
        platform::raise_priority();
        let injector: Arc<dyn InputInjector> = if opts.dry_run {
            log::info!("dry-run: nothing will be injected");
            Arc::new(RecordingInjector::logging())
        } else {
            platform::default_injector().map_err(|e| format!("no input injector: {e}"))?
        };

        let profiles_dir = opts.profiles_dir.clone().or_else(store::default_dir);
        // asked on its own thread: the engine thread (G13 keys) never waits for the system
        let watcher = opts.auto_switch.then(|| -> Box<dyn platform::ForegroundWatcher> {
            Box::new(platform::BackgroundWatcher::spawn(
                platform::default_foreground_watcher(),
                std::time::Duration::from_millis(100),
            ))
        });
        let mut profiles = ProfileManager::new(open_store(profiles_dir.as_deref()), watcher);
        let recent = profiles.recent_apps();

        let shutdown = Arc::new(AtomicBool::new(false));
        let (event_tx, event_rx) = channel();
        let (cmd_tx, cmd_rx) = channel();
        let cfg = HidConfig {
            read_mode: if opts.poll { ReadMode::Poll } else { ReadMode::Interrupt },
            ..HidConfig::default()
        };
        let hid = HidWorker::spawn(cfg, event_tx, cmd_rx, shutdown.clone())
            .map_err(|e| format!("could not start HID thread: {e}"))?;

        let profile = profiles.initial_profile();
        log::info!(
            "profile '{}' ({}){}",
            profiles.active_id(),
            profile.name,
            if opts.auto_switch { "" } else { ", auto-switch off" }
        );
        let mut engine = Engine::new(injector.clone(), cmd_tx, profile);
        engine.set_overlay_key(opts.overlay_key);
        let mut lcd = LcdManager::new(engine.subscribe());
        let lcd_lang = lcd.language();
        let ui = engine.subscribe();
        engine.emit(UiEvent::ActiveProfile(profiles.active_id().to_string()));

        let (req_tx, req_rx) = channel::<Request>();
        // foreground executable, for the mouse thread (real-time profiles per application)
        let focus: crate::mouse::Focus = Arc::default();
        let engine_focus = focus.clone();
        let mouse = if opts.mouse {
            let (mouse_tx, mouse_rx) = channel();
            let to_engine = std::sync::Mutex::new(req_tx.clone());
            let publish = move |st| {
                let _ = to_engine.lock().unwrap().send(Request::Emit(UiEvent::Mouse(Box::new(st))));
            };
            match MouseWorker::spawn(mouse_rx, publish, shutdown.clone(), injector.clone(), focus.clone()) {
                Ok(h) => Some((mouse_tx, h)),
                Err(e) => {
                    log::error!("could not start mouse thread: {e}");
                    None
                }
            }
        } else {
            None
        };
        let headset = if opts.headset {
            let (tx, rx) = channel();
            let to_engine = std::sync::Mutex::new(req_tx.clone());
            let publish = move |st| {
                let _ = to_engine.lock().unwrap().send(Request::Emit(UiEvent::Headset(Box::new(st))));
            };
            match HeadsetWorker::spawn(rx, publish, shutdown.clone(), focus.clone()) {
                Ok(h) => Some((tx, h)),
                Err(e) => {
                    log::error!("could not start headset thread: {e}");
                    None
                }
            }
        } else {
            None
        };
        let engine_shutdown = shutdown.clone();
        let engine_thread = thread::Builder::new()
            .name("g13-engine".into())
            .spawn(move || {
                engine.run_with(&event_rx, &engine_shutdown, |e| {
                    while let Ok(req) = req_rx.try_recv() {
                        match req {
                            Request::RescanProfiles => profiles.rescan(e),
                            Request::Preview(id) => profiles.set_preview(id),
                            Request::SetOverlayKey(k) => e.set_overlay_key(k),
                            Request::Pin(id) => profiles.set_pinned(id),
                            Request::Emit(ev) => e.emit(ev),
                        }
                    }
                    profiles.tick(e);
                    // G13 Studio's own window (and its overlay) does not change the mouse's profile
                    if let Ok(mut f) = engine_focus.lock()
                        && f.as_deref() != profiles.foreground_exe()
                        && !profiles.foreground_exe().is_some_and(|e| profiles.is_self(e))
                    {
                        *f = profiles.foreground_exe().map(str::to_string);
                    }
                    lcd.tick(e, &mut profiles);
                });
                // Also stop the HID thread if the engine ended on its own.
                engine_shutdown.store(true, Ordering::Relaxed);
            })
            .map_err(|e| format!("could not start engine thread: {e}"))?;

        Ok((
            Self {
                shutdown,
                recent,
                requests: req_tx,
                profiles_dir,
                lcd_lang,
                engine: Some(engine_thread),
                hid: Some(hid),
                mouse,
                headset,
            },
            ui,
        ))
    }

    /// Directory the profiles are read from (`None` = built-in profile only).
    pub fn profiles_dir(&self) -> Option<&Path> {
        self.profiles_dir.as_deref()
    }

    /// Applications recently in the foreground, most recent first.
    pub fn recent_apps(&self) -> Vec<crate::platform::ForegroundApp> {
        self.recent.lock().unwrap().clone()
    }

    pub fn request(&self, req: Request) {
        let _ = self.requests.send(req);
    }

    /// Send a command to the mouse thread (ignored without a mouse thread).
    pub fn mouse(&self, cmd: MouseCommand) {
        if let Some((tx, _)) = &self.mouse {
            let _ = tx.send(cmd);
        }
    }

    /// Send a command to the headset thread (ignored without one).
    pub fn headset(&self, cmd: HeadsetCommand) {
        if let Some((tx, _)) = &self.headset {
            let _ = tx.send(cmd);
        }
    }

    /// Language of the LCD texts (`"fr"`, `"en"`, `"de"`, `"it"`, `"es"`; others
    /// are ignored). French until set; applied from the next frame.
    pub fn set_lcd_language(&self, code: &str) {
        if let Some(lang) = crate::lcd::i18n::Lang::from_code(code) {
            self.lcd_lang.set(lang);
        }
    }

    /// Flag that stops everything when set (e.g. from a Ctrl+C handler).
    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    /// `false` once the engine thread has exited.
    pub fn is_running(&self) -> bool {
        self.engine.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Stop both threads. The engine releases every held key and stops all
    /// macros before exiting.
    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.engine.take() {
            let _ = h.join();
        }
        if let Some(h) = self.hid.take() {
            let _ = h.join();
        }
        if let Some((_, h)) = self.mouse.take() {
            let _ = h.join();
        }
        if let Some((_, h)) = self.headset.take() {
            let _ = h.join();
        }
        platform::shutdown();
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn open_store(dir: Option<&Path>) -> ProfileStore {
    let Some(dir) = dir else {
        log::warn!("no config directory on this system; using the built-in profile");
        return ProfileStore::builtin();
    };
    match ProfileStore::open(dir) {
        Ok(s) => {
            log::info!("profiles: {} ({} loaded, default '{}')", dir.display(), s.ids().count(), s.default_id());
            s
        }
        Err(e) => {
            log::error!("cannot use profile directory {}: {e}; using the built-in profile", dir.display());
            ProfileStore::builtin()
        }
    }
}
