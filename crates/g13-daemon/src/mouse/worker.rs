//! The mouse thread: sole owner of the mouse's HID handle (like the G13's
//! HID thread). Finds the mouse, keeps a [`MouseStatus`] up to date (battery,
//! settings of the active onboard profile) and changes settings on request.
//!
//! Like G HUB, changes show at once without being saved: [`MouseCommand::Preview`]
//! sets the lighting (RGB effects 0x8071 under software control) and the DPI
//! (0x2201) in the mouse's RAM, while it stays in its own memory mode so that
//! its DPI / profile / G-shift buttons keep working. [`MouseCommand::Revert`]
//! (and closing G13 Studio) hands the lighting back and reloads the profile.
//!
//! Saving goes to the mouse's flash, so it only happens on an explicit
//! [`MouseCommand::Apply`], never while G HUB runs (it would overwrite the
//! mouse again), after a full memory backup made at the first connection,
//! and every written sector is read back.
//!
//! Real time ([`MouseCommand::SetHost`], see [`super::host`]): the onboard
//! profiles are off, the mouse is in host mode and this thread applies G13
//! Studio's own profiles (one per application) and carries out the buttons.
//! Leaving (and closing G13 Studio) gives the mouse back to its memory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hidapi::HidApi;
use logi_hidpp::features::{self as f, Battery, DpiChoice, OnboardInfo};
use logi_hidpp::features::{per_led, rgb};
use logi_hidpp::onboard::{Directory, Profile};
use serde::Serialize;

use super::host::{self, Effect, HostState, SoftProfile};
use super::link::Link;
use super::settings::{self, Lighting, MouseSettings};
use super::link::{Spot, find_at};
use super::{Mouse, find_mice};
use crate::engine::action::MacroMode;
use crate::engine::macros::{self, MacroHandle};
use crate::platform::InputInjector;
use crate::profiles::format::macro_from_dto;

/// Executable in the foreground, kept up to date by the engine thread.
pub type Focus = Arc<Mutex<Option<String>>>;

/// Look for a mouse this often while none is connected. Not much more often: a
/// sleeping LIGHTSPEED mouse pinged every 150 ms never stays asleep (measured).
const DISCOVERY: Duration = Duration::from_secs(3);
/// Once a mouse was seen, only its own place is checked, except one round in this
/// many, which searches every receiver (the mouse may have moved to a cable…).
const FULL_SEARCH: u32 = 5;
const BATTERY_EVERY: Duration = Duration::from_secs(60);
/// The profile button may change the active profile at any time.
const PROFILE_EVERY: Duration = Duration::from_secs(2);
const GHUB_EVERY: Duration = Duration::from_secs(5);
/// Frame interval of the software gradient (a slow drift: 10 frames / s is smooth enough).
const FRAME: Duration = Duration::from_millis(100);
const GHUB_OPEN: &str =
    "G HUB est ouvert : il reprendrait la main sur la souris. Fermez-le, puis réessayez.";
/// Consecutive failed requests before the mouse counts as gone (asleep, out of range…).
const MAX_FAILURES: u32 = 3;
/// In real time: how long to wait for a button event per loop turn.
const EVENT_WAIT: Duration = Duration::from_millis(8);
const FOCUS_EVERY: Duration = Duration::from_millis(400);
const HOST_ON: &str = "Les profils intégrés sont désactivés : réactivez-les d'abord (Paramètres avancés › Profils intégrés).";
/// MouseButtonSpy: buttons reported as events, remappable (G HUB's real-time mode).
const BUTTON_SPY: u16 = 0x8110;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct MouseStatus {
    pub connected: bool,
    pub name: String,
    pub battery: Option<u8>,
    pub charging: bool,
    /// Settings of the active onboard profile (`None` while a program drives the mouse).
    pub settings: Option<MouseSettings>,
    /// Onboard profile in use (1..5); 0 = host mode (driven by a program such as G HUB).
    pub profile: u16,
    pub current_dpi: Option<u16>,
    pub dpi_min: u16,
    pub dpi_max: u16,
    pub dpi_step: u16,
    /// G HUB is running: it drives the mouse and would undo our changes.
    pub ghub_running: bool,
    /// Outcome of the last write (`Ok` message or error), for the GUI.
    pub message: Option<String>,
    pub error: bool,
    /// The mouse's onboard profiles (directory order): which are enabled, what they hold.
    pub profiles: Vec<OnboardSlot>,
    /// Real time: onboard profiles off, G13 Studio's profiles in use.
    pub host: bool,
    /// G13 Studio's own mouse profiles (the first one serves every application).
    pub soft_profiles: Vec<SoftProfile>,
    /// The one in use now (real time only).
    pub soft_active: Option<usize>,
    /// The one used where no application has its own.
    pub soft_default: usize,
    /// Tested everywhere ("Tester partout"), until stopped.
    pub soft_pinned: Option<usize>,
    /// Real time: profile slots of the buttons held right now (shown on the drawing).
    pub pressed: Vec<usize>,
}

/// One onboard profile of the mouse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardSlot {
    /// Sector = profile number shown (1..5).
    pub sector: u16,
    /// Reachable with the mouse's profile button (and selectable).
    pub enabled: bool,
    pub settings: Option<MouseSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseCommand {
    /// Write these settings into onboard profile `sector` (`None` = the active one).
    Apply {
        sector: Option<u16>,
        settings: MouseSettings,
    },
    /// Make this onboard profile the active one (it must be enabled).
    Select(u16),
    /// Enable or disable an onboard profile (the profile button skips disabled ones).
    SetEnabled { sector: u16, enabled: bool },
    /// Show this lighting (and DPI, if given) now, without saving anything.
    Preview {
        lighting: Lighting,
        dpi: Option<u16>,
    },
    /// Drop the previewed changes: the mouse shows its profile again.
    Revert,
    /// Onboard profiles off (`true`: real time with G13 Studio's profiles) or back on.
    SetHost(bool),
    /// Save G13 Studio profile `index` (`None` = add it at the end).
    SaveSoft {
        index: Option<usize>,
        profile: SoftProfile,
    },
    DeleteSoft(usize),
    /// G13 Studio profile used where no application has its own.
    SetSoftDefault(usize),
    /// "Tester partout": this profile in every application until `None`.
    PinSoft(Option<usize>),
}

pub struct MouseWorker {
    commands: Receiver<MouseCommand>,
    publish: Box<dyn Fn(MouseStatus) + Send>,
    shutdown: Arc<AtomicBool>,
    ghub: GhubWatch,
    injector: Arc<dyn InputInjector>,
    focus: Focus,
    cfg: HostFile,
    /// "Tester partout" (not saved: G13 Studio starts without it).
    pinned: Option<usize>,
}

impl MouseWorker {
    pub fn spawn(
        commands: Receiver<MouseCommand>,
        publish: impl Fn(MouseStatus) + Send + 'static,
        shutdown: Arc<AtomicBool>,
        injector: Arc<dyn InputInjector>,
        focus: Focus,
    ) -> std::io::Result<JoinHandle<()>> {
        let mut w = Self {
            commands,
            publish: Box::new(publish),
            shutdown,
            ghub: GhubWatch::new(),
            injector,
            focus,
            cfg: HostFile::load(),
            pinned: None,
        };
        thread::Builder::new()
            .name("mouse".into())
            .spawn(move || w.run())
    }

    fn stopping(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Sleep up to `d`, waking up early on shutdown.
    fn nap(&self, d: Duration) -> bool {
        let end = Instant::now() + d;
        while Instant::now() < end {
            if self.stopping() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.stopping()
    }

    fn run(&mut self) {
        let mut api = loop {
            match HidApi::new() {
                Ok(a) => break a,
                Err(e) => {
                    log::error!("mouse: hidapi init failed: {e}");
                    if self.nap(DISCOVERY) {
                        return;
                    }
                }
            }
        };
        let mut announced = false;
        // where the last mouse was (it is most likely asleep there) and the rounds since
        let mut last: Option<Spot> = None;
        let mut round = 0u32;
        while !self.stopping() {
            if let Err(e) = api.refresh_devices() {
                log::debug!("mouse: hidapi refresh failed: {e}");
            }
            // A woken mouse is taken back at the next round. Searching every receiver
            // took ~2 s more per round (the other receivers and the headset dongle are
            // probed too): only one round in FULL_SEARCH does it once a mouse was seen.
            round += 1;
            let found = match &last {
                Some(spot) if !round.is_multiple_of(FULL_SEARCH) => find_at(&api, spot),
                _ => find_mice(&api).into_iter().next(),
            };
            match found {
                Some(m) => {
                    log::info!("mouse connected: {} ({:04X})", m.name, m.product_id);
                    announced = false;
                    last = Some(m.spot.clone());
                    round = 0;
                    self.session(m);
                    log::info!("mouse disconnected (asleep, off or unplugged)");
                    let st = self.absent();
                    (self.publish)(st);
                }
                None if !announced => {
                    announced = true;
                    let st = self.absent();
                    (self.publish)(st);
                }
                None => {}
            }
            // commands sent while no mouse is there: only G13 Studio's profiles can change
            let mut edited = false;
            while let Ok(c) = self.commands.try_recv() {
                edited |= self.edit_soft(&c).is_some();
            }
            if edited {
                let st = self.absent();
                (self.publish)(st);
            }
            if self.nap(DISCOVERY) {
                return;
            }
        }
    }

    /// Status while no mouse is there (G13 Studio's profiles can still be edited).
    fn absent(&mut self) -> MouseStatus {
        MouseStatus {
            ghub_running: self.ghub.running(),
            host: self.cfg.host,
            soft_profiles: self.cfg.profiles.clone(),
            soft_default: self.cfg.default,
            soft_pinned: self.pinned,
            ..MouseStatus::default()
        }
    }

    /// Status read from the mouse, plus the real-time part.
    fn status(&mut self, s: &mut Session, ghub_running: bool) -> MouseStatus {
        let mut st = s.read_status(ghub_running);
        st.host = self.cfg.host;
        st.soft_profiles = self.cfg.profiles.clone();
        st.soft_default = self.cfg.default;
        st.soft_pinned = self.pinned;
        if let Some(h) = &s.host {
            st.soft_active = Some(h.active);
            st.settings = self.cfg.profiles.get(h.active).map(|p| p.settings.clone());
        }
        st
    }

    fn focus_exe(&self) -> Option<String> {
        self.focus.lock().ok().and_then(|f| f.clone())
    }

    /// Changes to G13 Studio's own profiles (saved in `mouse-profiles.json`).
    /// `None`: not such a command; `Some(Err)`: refused.
    fn edit_soft(&mut self, c: &MouseCommand) -> Option<Result<String, String>> {
        let r = match c {
            MouseCommand::SaveSoft { index, profile } => {
                let p = profile.clone();
                match index {
                    Some(i) if *i < self.cfg.profiles.len() => {
                        self.cfg.profiles[*i] = p;
                        Ok(format!("Profil {} enregistré", i + 1))
                    }
                    Some(_) => Err("profil inconnu".to_string()),
                    None if self.cfg.profiles.len() >= MAX_SOFT => {
                        Err(format!("{MAX_SOFT} profils au plus"))
                    }
                    None => {
                        self.cfg.profiles.push(p);
                        Ok(format!("Profil {} créé", self.cfg.profiles.len()))
                    }
                }
            }
            MouseCommand::DeleteSoft(i) if *i == self.cfg.default => Err(
                "c'est le profil par défaut : choisissez-en un autre par défaut d'abord".to_string(),
            ),
            MouseCommand::DeleteSoft(i) if *i < self.cfg.profiles.len() => {
                let i = *i;
                self.cfg.profiles.remove(i);
                if self.cfg.default > i {
                    self.cfg.default -= 1;
                }
                self.pinned = match self.pinned {
                    Some(p) if p == i => None,
                    Some(p) if p > i => Some(p - 1),
                    p => p,
                };
                Ok(format!("Profil {} supprimé", i + 1))
            }
            MouseCommand::DeleteSoft(_) => Err("profil inconnu".to_string()),
            MouseCommand::SetSoftDefault(i) if *i < self.cfg.profiles.len() => {
                self.cfg.default = *i;
                Ok(format!("Profil {} : profil par défaut", i + 1))
            }
            MouseCommand::PinSoft(p) if p.is_none_or(|p| p < self.cfg.profiles.len()) => {
                self.pinned = *p;
                // nothing to save
                return Some(Ok(String::new()));
            }
            MouseCommand::SetSoftDefault(_) | MouseCommand::PinSoft(_) => {
                Err("profil inconnu".to_string())
            }
            _ => return None,
        };
        if r.is_ok() {
            self.cfg.save();
        }
        Some(r)
    }

    fn session(&mut self, m: Mouse) {
        let Some(s) = Session::open(m) else { return };
        let mut s = s;
        s.backup_once();
        if self.cfg.host && !self.cfg.profiles.is_empty() {
            let exe = self.focus_exe();
            let choice = (self.cfg.default, self.pinned);
            if let Err(e) = s.enter_host(&self.cfg.profiles, exe, self.injector.clone(), choice) {
                log::warn!("mouse: real time: {e}");
            }
        }
        let ghub = self.ghub.running();
        let mut status = self.status(&mut s, ghub);
        (self.publish)(status.clone());

        let (mut battery_at, mut profile_at, mut ghub_at) =
            (Instant::now(), Instant::now(), Instant::now());
        let (mut focus_at, mut frame_at) = (Instant::now(), Instant::now());
        loop {
            if self.stopping() {
                // never leave the mouse on a preview or in real time: back to its own profile
                s.leave_host(&*self.injector, &self.cfg.profiles);
                s.revert_live();
                return;
            }
            if s.failures >= MAX_FAILURES {
                // gone: whatever its buttons held is let go
                s.release_held(&*self.injector, &self.cfg.profiles);
                return;
            }
            if s.host.is_some() {
                let before = s.host.as_ref().map(|h| h.active);
                s.pump(&*self.injector, &self.cfg.profiles, EVENT_WAIT);
                if s.host.as_ref().map(|h| h.active) != before {
                    status = self.status(&mut s, status.ghub_running);
                    (self.publish)(status.clone());
                } else if s.pressed() != status.pressed {
                    status.pressed = s.pressed();
                    (self.publish)(status.clone());
                }
            }
            let wait = if s.host.is_some() {
                Duration::ZERO
            } else if s.animating() {
                FRAME
            } else {
                Duration::from_millis(250)
            };
            let first = self.commands.recv_timeout(wait);
            // a slider sends many previews: only the last one of a burst matters
            let mut cmds: Vec<MouseCommand> = Vec::new();
            match first {
                Ok(c) => cmds.push(c),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            cmds.extend(self.commands.try_iter());
            let mut kept: Vec<MouseCommand> = Vec::new();
            for c in cmds {
                if matches!(c, MouseCommand::Preview { .. })
                    && matches!(kept.last(), Some(MouseCommand::Preview { .. }))
                {
                    kept.pop();
                }
                kept.push(c);
            }
            for cmd in kept {
                match cmd {
                    MouseCommand::Preview { lighting, dpi } => {
                        if let Err(e) = s.preview(&lighting, dpi) {
                            log::warn!("mouse preview: {e}");
                        }
                    }
                    MouseCommand::Revert => {
                        if s.host.is_some() {
                            s.reapply(&*self.injector, &self.cfg.profiles);
                        } else {
                            s.revert_live();
                        }
                        let keep = status.ghub_running;
                        status = self.status(&mut s, keep);
                        (self.publish)(status.clone());
                    }
                    MouseCommand::SetHost(on) => {
                        let ghub = self.ghub.running();
                        let r = if ghub {
                            Err(GHUB_OPEN.to_string())
                        } else if on {
                            if self.cfg.profiles.is_empty() {
                                // profile 1 starts as a copy of what the mouse does now
                                let settings = status.settings.clone();
                                self.cfg.profiles.push(SoftProfile {
                                    name: String::new(),
                                    apps: Vec::new(),
                                    settings: settings.unwrap_or_else(default_settings),
                                });
                            }
                            let exe = self.focus_exe();
                            let choice = (self.cfg.default, self.pinned);
                            s.enter_host(&self.cfg.profiles, exe, self.injector.clone(), choice)
                                .map(|()| "Temps réel : G13 Studio pilote la souris".to_string())
                        } else {
                            s.leave_host(&*self.injector, &self.cfg.profiles);
                            Ok("Profils intégrés de la souris réactivés".to_string())
                        };
                        if r.is_ok() {
                            self.cfg.host = on;
                            self.cfg.save();
                        }
                        status = self.status(&mut s, ghub);
                        match r {
                            Ok(m) => status.message = Some(m),
                            Err(e) => {
                                status.message = Some(e);
                                status.error = true;
                            }
                        }
                        (self.publish)(status.clone());
                    }
                    c @ (MouseCommand::SaveSoft { .. }
                    | MouseCommand::DeleteSoft(_)
                    | MouseCommand::SetSoftDefault(_)
                    | MouseCommand::PinSoft(_)) => {
                        let (save, removed) = match &c {
                            MouseCommand::SaveSoft { profile, .. } => (
                                profile_check(&profile.settings, s.dpi_range),
                                None,
                            ),
                            MouseCommand::DeleteSoft(i) => (Ok(()), Some(*i)),
                            _ => (Ok(()), None),
                        };
                        let r = save.and_then(|()| self.edit_soft(&c).unwrap_or(Ok(String::new())));
                        if r.is_ok() && s.host.is_some() {
                            let edited = match &c {
                                MouseCommand::SaveSoft { index: Some(i), .. } => Some(*i),
                                _ => None,
                            };
                            s.set_choice(self.cfg.default, self.pinned);
                            s.soft_changed(&*self.injector, &self.cfg.profiles, edited, removed);
                        }
                        status = self.status(&mut s, status.ghub_running);
                        match r {
                            Ok(m) => status.message = Some(m),
                            Err(e) => {
                                status.message = Some(e);
                                status.error = true;
                            }
                        }
                        (self.publish)(status.clone());
                    }
                    MouseCommand::Select(_) | MouseCommand::SetEnabled { .. } | MouseCommand::Apply { .. }
                        if s.host.is_some() =>
                    {
                        status.message = Some(HOST_ON.to_string());
                        status.error = true;
                        (self.publish)(status.clone());
                    }
                    MouseCommand::Select(sector) => {
                        let ghub = self.ghub.running();
                        let r = s.select(sector);
                        status = self.status(&mut s, ghub);
                        if let Err(e) = r {
                            status.message = Some(e);
                            status.error = true;
                        }
                        (self.publish)(status.clone());
                    }
                    MouseCommand::SetEnabled { sector, enabled } => {
                        let ghub = self.ghub.running();
                        let r = if ghub {
                            Err(GHUB_OPEN.to_string())
                        } else {
                            s.set_enabled(sector, enabled)
                        };
                        status = self.status(&mut s, ghub);
                        if let Err(e) = r {
                            status.message = Some(e);
                            status.error = true;
                        }
                        (self.publish)(status.clone());
                    }
                    MouseCommand::Apply {
                        sector,
                        settings: new,
                    } => {
                        let ghub = self.ghub.running();
                        let r = s.apply(sector, &new, ghub);
                        status = self.status(&mut s, ghub);
                        match r {
                            Ok(msg) => status.message = Some(msg),
                            Err(e) => {
                                log::warn!("mouse: {e}");
                                status.message = Some(e);
                                status.error = true;
                            }
                        }
                        (self.publish)(status.clone());
                    }
                }
            }
            let now = Instant::now();
            if s.animating() && !status.ghub_running && now >= frame_at {
                frame_at = now + FRAME;
                s.draw_frame();
            }
            let mut changed = false;
            if s.host.is_some() && now >= focus_at {
                focus_at = now + FOCUS_EVERY;
                let exe = self.focus_exe();
                if s.focus_changed(&*self.injector, &self.cfg.profiles, exe) {
                    status = self.status(&mut s, status.ghub_running);
                    changed = true;
                }
            }
            if now >= ghub_at {
                ghub_at = now + GHUB_EVERY;
                let g = self.ghub.running();
                changed |= g != status.ghub_running;
                status.ghub_running = g;
            }
            if now >= profile_at {
                profile_at = now + PROFILE_EVERY;
                if let Some(p) = s.current_profile()
                    && p != status.profile
                {
                    // switched with the mouse's profile button (or by another program)
                    let keep = (status.message.take(), status.error);
                    status = self.status(&mut s, status.ghub_running);
                    (status.message, status.error) = keep;
                    changed = true;
                }
            }
            if now >= battery_at {
                battery_at = now + BATTERY_EVERY;
                if let Some((pct, chg)) = s.battery() {
                    changed |= status.battery != Some(pct) || status.charging != chg;
                    status.battery = Some(pct);
                    status.charging = chg;
                }
            }
            if changed {
                (self.publish)(status.clone());
            }
        }
    }
}

/// Per lighting cluster: its (effect index, effect id) list.
type RgbZones = Vec<(u8, Vec<(u8, u16)>)>;

/// An open mouse and the feature indices we use.
struct Session {
    link: Link,
    name: String,
    oi: u8,
    info: OnboardInfo,
    dpi_index: Option<u8>,
    battery_index: Option<u8>,
    dpi_range: (u16, u16, u16),
    failures: u32,
    /// RGB effects feature index and, per cluster, (effect index, effect id).
    rgb: Option<(u8, RgbZones)>,
    /// We hold software control of the lighting (a preview is shown).
    rgb_claimed: bool,
    /// The DPI was changed live (the profile reload puts it back).
    dpi_live: bool,
    /// Factory ROM profile: source of the factory lighting.
    factory: Option<Profile>,
    /// Per-LED lighting feature index and LED ids (0x8081).
    per_led: Option<(u8, Vec<u8>)>,
    /// The firmware effect is off and the LEDs are ours (per-LED frames).
    per_led_on: bool,
    /// Software gradient saved by the user (`mouse.toml`): drawn whenever G13 Studio runs.
    soft: Option<Lighting>,
    /// Real time is on (host mode, buttons spied).
    host: Option<HostRun>,
    /// Onboard profiles as last read.
    onboard_cache: Vec<OnboardSlot>,
    /// Gradient drawn right now (the saved one, or a preview).
    anim: Option<Lighting>,
    anim_start: Instant,
}

impl Session {
    fn open(m: Mouse) -> Option<Self> {
        let link = m.link;
        let oi = link.feature(f::ONBOARD_PROFILES)?;
        let info = OnboardInfo::parse(&link.call(oi, 0, &[]).ok()?);
        let dpi_index = link.feature(f::ADJUSTABLE_DPI);
        let battery_index = link.feature(f::UNIFIED_BATTERY);
        let mut dpi_range = (100, 25600, 50);
        if let Some(i) = dpi_index
            && let Ok(d) = link.call(i, 1, &[0])
        {
            let words: Vec<u16> = d[1..]
                .chunks(2)
                .filter(|c| c.len() == 2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let list = f::dpi_list(&words);
            if let Some(DpiChoice::Range { min, max, step }) = list.first() {
                dpi_range = (*min, *max, *step);
            } else if let (Some(DpiChoice::Value(a)), Some(DpiChoice::Value(b))) =
                (list.first(), list.last())
            {
                dpi_range = (*a, *b, 1);
            }
        }
        let rgb = link.feature(f::RGB_EFFECTS).and_then(|i| {
            let clusters = link.call(i, 0, &[0xFF, 0xFF, 0x00]).ok()?[2];
            let mut zones = Vec::new();
            for c in 0..clusters {
                let n = link.call(i, 0, &[c, 0xFF, 0x00]).ok()?[4];
                let effects = (0..n)
                    .filter_map(|e| {
                        link.call(i, 0, &[c, e, 0x00])
                            .ok()
                            .map(|d| (e, u16::from_be_bytes([d[2], d[3]])))
                    })
                    .collect();
                zones.push((c, effects));
            }
            Some((i, zones))
        });
        let per_led = link_per_led(&link);
        // the factory profile is the first ROM sector that holds one (0x0101 on the G502 X PLUS)
        let factory = [f::rom_sector(1), f::rom_sector(0)]
            .into_iter()
            .find_map(|s| {
                let p =
                    Profile::parse(&link.read_sector(oi, s, info.sector_size).ok()?, false).ok()?;
                (p.custom_animation() != 0 || p.dpi(0).is_some_and(|d| (100..=26000).contains(&d)))
                    .then_some(p)
            });
        Some(Self {
            link,
            name: m.name,
            oi,
            info,
            dpi_index,
            battery_index,
            dpi_range,
            failures: 0,
            rgb,
            rgb_claimed: false,
            dpi_live: false,
            factory,
            per_led,
            per_led_on: false,
            soft: load_soft(),
            host: None,
            onboard_cache: Vec::new(),
            anim: None,
            anim_start: Instant::now(),
        })
        .map(|mut s: Self| {
            s.anim = s.soft.clone();
            s
        })
    }

    fn animating(&self) -> bool {
        self.anim.is_some() && self.per_led.is_some()
    }

    /// One frame of the gradient on every LED.
    fn draw_frame(&mut self) {
        let (
            Some(Lighting::Gradient {
                from,
                to,
                period_ms,
                reverse,
            }),
            Some((pk, leds)),
        ) = (self.anim.clone(), self.per_led.clone())
        else {
            return;
        };
        let (Ok(a), Ok(b)) = (settings::parse_color(&from), settings::parse_color(&to)) else {
            return;
        };
        if !self.per_led_on && self.enter_per_led().is_err() {
            return;
        }
        let t = self.anim_start.elapsed().as_millis() as u64;
        let colors = settings::gradient_frame(a, b, leds.len(), t, period_ms, reverse);
        let frame: Vec<(u8, [u8; 3])> = leds.iter().copied().zip(colors).collect();
        for req in per_led::set_requests(&frame) {
            if self.call(pk, per_led::FN_SET, &req).is_err() {
                return;
            }
        }
        let _ = self.call(pk, per_led::FN_FRAME_END, &per_led::FRAME_END);
    }

    /// Take the lighting and switch the firmware effect off so per-LED colours show.
    fn enter_per_led(&mut self) -> Result<(), String> {
        let Some((ri, zones)) = self.rgb.clone() else {
            return Err("pas de 0x8071".into());
        };
        if !self.rgb_claimed {
            self.call(ri, 5, &rgb::SW_CONTROL_TAKE)?;
            self.rgb_claimed = true;
        }
        let n = zones.first().map_or(0, |(_, e)| e.len() as u8);
        self.call(ri, 1, &[0xFF, n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01])?;
        self.per_led_on = true;
        Ok(())
    }

    /// Show `lighting` (and `dpi`) now, in RAM only.
    fn preview(&mut self, lighting: &Lighting, dpi: Option<u16>) -> Result<(), String> {
        if let (Some(d), Some(i)) = (dpi, self.dpi_index) {
            let (min, max, _) = self.dpi_range;
            let d = d.clamp(min, max);
            self.call(i, 3, &[0, (d >> 8) as u8, d as u8])?;
            self.dpi_live = true;
        }
        if let Lighting::Gradient { .. } = lighting {
            self.anim = Some(lighting.clone());
            self.draw_frame();
            return Ok(());
        }
        self.anim = None;
        self.per_led_on = false;
        let Some((ri, zones)) = self.rgb.clone() else {
            return Ok(());
        };
        match settings::live_effect(lighting)? {
            // the mouse's own lighting (factory gradient…): give it back
            None => self.release_rgb(),
            Some(effect) => {
                if !self.rgb_claimed {
                    self.call(ri, 5, &rgb::SW_CONTROL_TAKE)?;
                    self.rgb_claimed = true;
                }
                for (cluster, effects) in &zones {
                    if let Some((idx, _)) = effects.iter().find(|(_, id)| *id == effect.effect_id())
                    {
                        self.call(ri, 1, &rgb::cluster_effect(*cluster, *idx, &effect))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn release_rgb(&mut self) {
        if self.rgb_claimed
            && let Some((ri, _)) = self.rgb
        {
            let _ = self.call(ri, 5, &rgb::SW_CONTROL_RELEASE);
            self.rgb_claimed = false;
        }
        self.per_led_on = false;
    }

    /// Drop every preview: lighting back to the mouse, profile reloaded (DPI).
    fn revert_live(&mut self) {
        self.release_rgb();
        self.anim = self.soft.clone();
        if self.dpi_live {
            self.dpi_live = false;
            if let Some(p) = self.current_profile().filter(|&p| p != 0) {
                let _ = self.call(self.oi, 3, &[(p >> 8) as u8, p as u8]);
            }
        }
    }

    /// Host mode, profile for `exe` applied, buttons spied.
    fn enter_host(
        &mut self,
        profiles: &[SoftProfile],
        exe: Option<String>,
        inj: Arc<dyn InputInjector>,
        (default, pinned): (usize, Option<usize>),
    ) -> Result<(), String> {
        if self.host.is_some() {
            return Ok(());
        }
        let si = self
            .link
            .feature(BUTTON_SPY)
            .ok_or("cette souris ne signale pas ses boutons (0x8110) : temps réel impossible")?;
        let active = host::choose(profiles, exe.as_deref(), default, pinned);
        let p = profiles.get(active).ok_or("aucun profil G13 Studio")?;
        self.call(self.oi, 1, &[2])?;
        self.host = Some(HostRun {
            si,
            active,
            exe,
            state: HostState::new(&p.settings),
            inj,
            macros: Vec::new(),
            default,
            pinned,
        });
        let r = self.apply_soft(&p.settings).and_then(|()| self.call(si, 1, &[]).map(drop));
        if let Err(e) = r {
            self.host = None;
            let _ = self.call(si, 4, &host::IDENTITY);
            let _ = self.call(self.oi, 1, &[1]);
            return Err(e);
        }
        log::info!("mouse: real time on (profile {})", active + 1);
        Ok(())
    }

    /// Settings of a G13 Studio profile, in RAM: rate, DPI, lighting, remap table.
    fn apply_soft(&mut self, s: &MouseSettings) -> Result<(), String> {
        let Some(si) = self.host.as_ref().map(|h| h.si) else { return Ok(()) };
        if let Some(i) = self.link.feature(f::REPORT_RATE) {
            let _ = self.call(i, 2, &[(1000 / s.report_rate_hz.max(125)) as u8]);
        }
        let dpi = s.dpi.get(usize::from(s.default_slot)).copied();
        self.preview(&s.lighting, dpi)?;
        self.dpi_live = false;
        self.call(si, 4, &host::remap_table(s))?;
        Ok(())
    }

    /// Let go of whatever the buttons hold (keys, clicks).
    fn release_held(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile]) {
        let Some(h) = self.host.as_mut() else { return };
        // a cancelled macro releases whatever it pressed
        for m in h.macros.drain(..) {
            m.handle.cancel();
        }
        if let Some(p) = profiles.get(h.active) {
            for e in h.state.release_all(&p.settings) {
                run_effect(inj, &e);
            }
        }
    }

    /// Back to the mouse's own memory (buttons, DPI, lighting of its profile).
    fn leave_host(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile]) {
        self.release_held(inj, profiles);
        let Some(h) = self.host.take() else { return };
        let _ = self.call(h.si, 2, &[]);
        let _ = self.call(h.si, 4, &host::IDENTITY);
        let _ = self.call(self.oi, 1, &[1]);
        self.release_rgb();
        self.anim = self.soft.clone();
        self.dpi_live = false;
        if let Some(p) = self.current_profile().filter(|&p| p != 0) {
            let _ = self.call(self.oi, 3, &[(p >> 8) as u8, p as u8]);
        }
        log::info!("mouse: real time off");
    }

    /// Switch to G13 Studio profile `i` (held buttons are let go first).
    fn use_soft(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile], i: usize) {
        let Some(p) = profiles.get(i) else { return };
        self.release_held(inj, profiles);
        if let Some(h) = self.host.as_mut() {
            h.active = i;
            h.state = HostState::new(&p.settings);
        }
        if let Err(e) = self.apply_soft(&p.settings) {
            log::warn!("mouse: profile {}: {e}", i + 1);
        }
    }

    /// The active profile again, as saved (drops a preview).
    fn reapply(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile]) {
        if let Some(i) = self.host.as_ref().map(|h| h.active) {
            self.use_soft(inj, profiles, i);
        }
    }

    /// The profiles were edited: re-apply the active one if it changed or moved.
    fn soft_changed(
        &mut self,
        inj: &dyn InputInjector,
        profiles: &[SoftProfile],
        edited: Option<usize>,
        removed: Option<usize>,
    ) {
        let Some(active) = self.host.as_ref().map(|h| h.active) else { return };
        // a new or edited profile may now be the one of the application in the foreground
        let Some(h) = self.host.as_ref() else { return };
        let want = host::choose(profiles, h.exe.as_deref(), h.default, h.pinned);
        if want != active || edited == Some(active) || removed.is_some() {
            self.use_soft(inj, profiles, want);
        }
    }

    /// Default profile and "Tester partout" changed (applied by [`Session::soft_changed`]).
    fn set_choice(&mut self, default: usize, pinned: Option<usize>) {
        if let Some(h) = self.host.as_mut() {
            h.default = default;
            h.pinned = pinned;
        }
    }

    /// The foreground application changed: its profile (true if another one is used now).
    fn focus_changed(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile], exe: Option<String>) -> bool {
        let Some(h) = self.host.as_mut() else { return false };
        if h.exe == exe {
            return false;
        }
        h.exe = exe;
        let want = host::choose(profiles, h.exe.as_deref(), h.default, h.pinned);
        if want == h.active {
            return false;
        }
        self.use_soft(inj, profiles, want);
        true
    }

    /// Slots of the buttons held now (real time).
    fn pressed(&self) -> Vec<usize> {
        let Some(h) = &self.host else { return Vec::new() };
        host::SPY_SLOTS
            .iter()
            .enumerate()
            .filter(|(bit, _)| h.state.mask & (1 << bit) != 0)
            .map(|(_, &slot)| slot)
            .collect()
    }

    /// A macro button pressed: start (or, in toggle mode, stop) its macro, like the G13 does.
    fn macro_down(&mut self, name: &str, dto: &crate::profiles::format::MacroDto) {
        let Some(h) = self.host.as_mut() else { return };
        h.macros.retain(|m| !m.handle.is_finished());
        let mac = match macro_from_dto(name, dto) {
            Ok(m) => m,
            Err(e) => return log::warn!("mouse: {e}"),
        };
        let (looping, held, toggle) = match mac.mode {
            MacroMode::Once if h.macros.iter().any(|m| m.name == name) => return,
            MacroMode::Once => (false, false, false),
            MacroMode::RepeatWhileHeld => (true, true, false),
            MacroMode::Toggle => {
                if let Some(i) = h.macros.iter().position(|m| m.name == name && m.toggle) {
                    h.macros.remove(i).handle.cancel();
                    return;
                }
                (true, false, true)
            }
        };
        match macros::spawn(h.inj.clone(), Arc::new(mac), looping) {
            Ok(handle) => h.macros.push(RunningMacro { name: name.to_string(), held, toggle, handle }),
            Err(e) => log::error!("mouse: macro {name}: {e}"),
        }
    }

    /// A macro button released: a "repeat while held" macro stops.
    fn macro_up(&mut self, name: &str) {
        let Some(h) = self.host.as_mut() else { return };
        h.macros.retain(|m| {
            let stop = m.held && m.name == name;
            if stop {
                m.handle.cancel();
            }
            !stop && !m.handle.is_finished()
        });
    }

    /// Button events of the last `wait`, carried out.
    fn pump(&mut self, inj: &dyn InputInjector, profiles: &[SoftProfile], wait: Duration) {
        let Some(si) = self.host.as_ref().map(|h| h.si) else { return };
        let mut wait = wait;
        while let Some((fi, func, data)) = self.link.event(wait) {
            wait = Duration::ZERO;
            if fi != si || func != 0 {
                continue;
            }
            let Some(h) = self.host.as_mut() else { return };
            let Some(p) = profiles.get(h.active) else { return };
            let active = h.active;
            let effects = h.state.buttons(&p.settings, host::spy_mask(&data), h.active, profiles.len());
            for e in effects {
                match e {
                    Effect::Dpi(d) => {
                        if let Some(i) = self.dpi_index {
                            let _ = self.call(i, 3, &[0, (d >> 8) as u8, d as u8]);
                        }
                    }
                    Effect::Profile(i) => {
                        // chosen by hand: kept until the foreground application changes
                        self.use_soft(inj, profiles, i);
                        return;
                    }
                    Effect::MacroDown(name) => {
                        let dto = profiles.get(active).and_then(|p| p.settings.macros.get(&name)).cloned();
                        if let Some(dto) = dto {
                            self.macro_down(&name, &dto);
                        }
                    }
                    Effect::MacroUp(name) => self.macro_up(&name),
                    e => run_effect(inj, &e),
                }
            }
        }
    }

    fn call(&mut self, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16], String> {
        let r = self.link.call(feature_index, function, params);
        match &r {
            Ok(_) => self.failures = 0,
            Err(e) if e == "pas de réponse" => self.failures += 1,
            Err(_) => {}
        }
        r
    }

    fn current_profile(&mut self) -> Option<u16> {
        let mode = self.call(self.oi, 2, &[]).ok()?[0];
        if mode != 1 {
            return Some(0);
        }
        let d = self.call(self.oi, 4, &[]).ok()?;
        Some(u16::from_be_bytes([d[0], d[1]]))
    }

    fn battery(&mut self) -> Option<(u8, bool)> {
        let i = self.battery_index?;
        let b = Battery::parse(&self.call(i, 1, &[]).ok()?);
        Some((b.percent, matches!(b.charging, 1 | 2)))
    }

    fn read_status(&mut self, ghub_running: bool) -> MouseStatus {
        let profile = self.current_profile().unwrap_or(0);
        let settings = (profile != 0)
            .then(|| {
                self.link
                    .read_sector(self.oi, profile, self.info.sector_size)
                    .ok()
            })
            .flatten()
            .and_then(|b| Profile::parse(&b, true).ok())
            .map(|p| {
                let mut s = settings::from_profile(&p);
                if let Some(g) = &self.soft {
                    s.lighting = g.clone();
                }
                s
            });
        let current_dpi = self
            .dpi_index
            .and_then(|i| self.call(i, 2, &[0]).ok())
            .map(|d| u16::from_be_bytes([d[1], d[2]]));
        let (battery, charging) = self.battery().map_or((None, false), |(p, c)| (Some(p), c));
        let (dpi_min, dpi_max, dpi_step) = self.dpi_range;
        // in real time the onboard memory does not change: no need to read it again
        // (a profile switch must not hold the buttons up)
        let profiles = if self.host.is_some() && !self.onboard_cache.is_empty() {
            self.onboard_cache.clone()
        } else {
            let p = self.read_profiles(profile, settings.as_ref());
            self.onboard_cache.clone_from(&p);
            p
        };
        MouseStatus {
            profiles,
            connected: true,
            name: self.name.clone(),
            battery,
            charging,
            settings,
            profile,
            current_dpi,
            dpi_min,
            dpi_max,
            dpi_step,
            ghub_running,
            message: None,
            error: false,
            ..MouseStatus::default()
        }
    }

    /// Every onboard profile listed in the directory, with its settings
    /// (`current` / `current_settings`: already read, the gradient included).
    fn read_profiles(
        &mut self,
        current: u16,
        current_settings: Option<&MouseSettings>,
    ) -> Vec<OnboardSlot> {
        let Ok(dir) = self
            .link
            .read_sector(self.oi, 0, self.info.sector_size)
            .and_then(|b| Directory::parse(&b))
        else {
            return Vec::new();
        };
        dir.entries
            .iter()
            .map(|e| OnboardSlot {
                sector: e.sector,
                enabled: e.enabled,
                settings: if e.sector == current {
                    current_settings.cloned()
                } else {
                    self.link
                        .read_sector(self.oi, e.sector, self.info.sector_size)
                        .ok()
                        .and_then(|b| Profile::parse(&b, true).ok())
                        .map(|p| settings::from_profile(&p))
                },
            })
            .collect()
    }

    /// Make onboard profile `sector` the active one.
    fn select(&mut self, sector: u16) -> Result<(), String> {
        if self.call(self.oi, 2, &[])?[0] != 1 {
            self.call(self.oi, 1, &[1])?;
        }
        self.call(self.oi, 3, &[(sector >> 8) as u8, sector as u8])
            .map(drop)
            .map_err(|_| format!("le profil {sector} est désactivé : activez-le d'abord"))
    }

    /// Enable / disable a profile in the directory. The last enabled one stays;
    /// disabling the active one first switches to another enabled profile.
    fn set_enabled(&mut self, sector: u16, enabled: bool) -> Result<(), String> {
        if !self.backup_exists() {
            self.backup_once();
        }
        let mut dir =
            Directory::parse(&self.link.read_sector(self.oi, 0, self.info.sector_size)?)?;
        let Some(i) = dir.entries.iter().position(|e| e.sector == sector) else {
            return Err(format!("profil {sector} inconnu"));
        };
        if !enabled
            && dir
                .entries
                .iter()
                .filter(|e| e.enabled && e.sector != sector)
                .count()
                == 0
        {
            return Err("au moins un profil doit rester activé".into());
        }
        if dir.entries[i].enabled == enabled {
            return Ok(());
        }
        let current = self.current_profile().unwrap_or(0);
        if !enabled && current == sector {
            let other = dir
                .entries
                .iter()
                .find(|e| e.enabled && e.sector != sector)
                .map(|e| e.sector)
                .unwrap_or(1);
            self.select(other)?;
        }
        dir.entries[i].enabled = enabled;
        self.link.write_sector(self.oi, 0, &dir.to_bytes())?;
        log::info!(
            "mouse: onboard profile {sector} {}",
            if enabled { "enabled" } else { "disabled" }
        );
        Ok(())
    }

    /// Write `s` into onboard profile `target` (`None` = the active one); the
    /// active profile is reloaded so that the changes show at once.
    fn apply(
        &mut self,
        target: Option<u16>,
        s: &MouseSettings,
        ghub_running: bool,
    ) -> Result<String, String> {
        if ghub_running {
            return Err(GHUB_OPEN.into());
        }
        let (min, max, step) = self.dpi_range;
        settings::validate(s, min, max, step, false)?;
        if !self.backup_exists() {
            self.backup_once();
            if !self.backup_exists() {
                return Err("sauvegarde de la mémoire impossible : rien n'a été écrit".into());
            }
        }
        // a program left it in host mode: back to the mouse's own memory
        if self.call(self.oi, 2, &[])?[0] != 1 {
            self.call(self.oi, 1, &[1])?;
        }
        let current = self
            .current_profile()
            .filter(|&p| p != 0)
            .ok_or("aucun profil actif dans la souris")?;
        let profile = target.unwrap_or(current);
        let mut p = Profile::parse(
            &self
                .link
                .read_sector(self.oi, profile, self.info.sector_size)?,
            true,
        )?;
        settings::apply(&mut p, s, self.factory.as_ref())?;
        self.link.write_sector(self.oi, profile, &p.to_bytes())?;
        if profile != current {
            log::info!("mouse: settings written to onboard profile {profile} (not active)");
            return Ok(format!("Enregistré dans la souris (profil {profile})"));
        }
        // the gradient lives in G13 Studio (mouse.toml), the rest in the mouse
        self.soft = matches!(s.lighting, Lighting::Gradient { .. }).then(|| s.lighting.clone());
        save_soft(self.soft.as_ref());
        // what is shown now is what is saved: no more preview
        self.release_rgb();
        self.anim = self.soft.clone();
        self.dpi_live = false;
        // re-selecting the profile makes the mouse reload it (DPI, rate, lighting)
        self.call(self.oi, 3, &[(profile >> 8) as u8, profile as u8])?;
        log::info!("mouse: settings written to onboard profile {profile}");
        Ok(format!("Enregistré dans la souris (profil {profile})"))
    }

    fn slug(&self) -> String {
        self.name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }

    fn backups_dir() -> Option<PathBuf> {
        crate::profiles::store::default_dir().and_then(|p| p.parent().map(|p| p.join("backups")))
    }

    fn backup_exists(&self) -> bool {
        let prefix = format!("hidpp-{}-", self.slug());
        Self::backups_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .is_some_and(|mut it| {
                it.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(&prefix)))
            })
    }

    /// Copy the whole onboard memory once per mouse, before we ever write to it.
    fn backup_once(&mut self) {
        if self.backup_exists() {
            return;
        }
        let Some(root) = Self::backups_dir() else {
            return;
        };
        let dir = root.join(format!(
            "hidpp-{}-{}",
            self.slug(),
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
        ));
        let size = self.info.sector_size;
        let sectors: Vec<u16> = (0..u16::from(self.info.sector_count))
            .chain((0..u16::from(self.info.profile_count_oob)).map(f::rom_sector))
            .collect();
        let mut files = Vec::new();
        for s in sectors {
            match self.link.read_sector(self.oi, s, size) {
                Ok(d) => files.push((s, d)),
                Err(e) => {
                    log::warn!("mouse backup: sector 0x{s:04X}: {e}; backup abandoned");
                    return;
                }
            }
        }
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        for (s, d) in files {
            let _ = std::fs::write(dir.join(format!("sector_{s:04X}.bin")), d);
        }
        log::info!("mouse memory backed up to {}", dir.display());
    }
}

/// Real time in progress.
struct HostRun {
    /// MouseButtonSpy feature index.
    si: u8,
    /// G13 Studio profile in use.
    active: usize,
    /// Foreground executable it was chosen for.
    exe: Option<String>,
    state: HostState,
    /// Injector of G13 Studio (macros run on threads of their own).
    inj: Arc<dyn InputInjector>,
    /// Macros started by the buttons, still running.
    macros: Vec<RunningMacro>,
    /// Profile used where no application has its own.
    default: usize,
    /// "Tester partout".
    pinned: Option<usize>,
}

struct RunningMacro {
    name: String,
    /// "Repeat while held": stops when its button is released.
    held: bool,
    /// "Toggle": the next press stops it.
    toggle: bool,
    handle: MacroHandle,
}

/// G13 Studio profiles at most (as many as the onboard ones).
const MAX_SOFT: usize = 5;

/// Keys, clicks and scroll of a button, sent to the OS.
fn run_effect(inj: &dyn InputInjector, e: &Effect) {
    let r = match *e {
        Effect::Key(k, down) => inj.key(k, down),
        Effect::Mouse(b, down) => inj.mouse_button(b, down),
        Effect::HWheel(d) => inj.mouse_wheel(d, true),
        Effect::Dpi(_) | Effect::Profile(_) | Effect::MacroDown(_) | Effect::MacroUp(_) => Ok(()),
    };
    if let Err(e) = r {
        log::warn!("mouse: {e}");
    }
}

/// A G13 Studio profile's settings are possible on this mouse.
fn profile_check(s: &MouseSettings, (min, max, step): (u16, u16, u16)) -> Result<(), String> {
    settings::validate(s, min, max, step, true)
}

/// Settings of a new profile when the mouse could not be read.
fn default_settings() -> MouseSettings {
    let m = |b: &str| Some(settings::MouseAction::Mouse { mouse: b.into() });
    let sp = |n: &str| Some(settings::MouseAction::Special { special: n.into() });
    MouseSettings {
        dpi: vec![800, 1600, 3200],
        default_slot: 1,
        shift_slot: 0,
        report_rate_hz: 1000,
        lighting: Lighting::Signature,
        buttons: vec![
            m("Left"),
            m("Right"),
            m("Middle"),
            m("Back"),
            sp("dpi_shift"),
            m("Forward"),
            sp("wheel_left"),
            sp("wheel_right"),
            sp("profile_cycle"),
            sp("dpi_up"),
            sp("dpi_down"),
        ],
        gshift: vec![None; 11],
        macros: Default::default(),
    }
}

/// `<config>/mouse-profiles.json`: real time on / off and G13 Studio's mouse profiles
/// (JSON: a button may hold nothing, which TOML cannot write in a list).
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct HostFile {
    #[serde(default)]
    host: bool,
    #[serde(default)]
    profiles: Vec<SoftProfile>,
    /// Profile used where no application has its own (index).
    #[serde(default)]
    default: usize,
}

impl HostFile {
    fn path() -> Option<PathBuf> {
        crate::profiles::store::default_dir()
            .and_then(|p| p.parent().map(|p| p.join("mouse-profiles.json")))
    }

    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("mouse: {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("mouse: {e}"),
        }
    }
}

fn link_per_led(link: &Link) -> Option<(u8, Vec<u8>)> {
    let i = link.feature(per_led::FEATURE)?;
    let ids = per_led::led_ids(&link.call(i, per_led::FN_INFO, &[0, 0]).ok()?);
    (!ids.is_empty()).then_some((i, ids))
}

/// `<config>/mouse.toml`: what G13 Studio itself draws on the mouse (software gradient).
fn soft_path() -> Option<PathBuf> {
    crate::profiles::store::default_dir().and_then(|p| p.parent().map(|p| p.join("mouse.toml")))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SoftFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lighting: Option<Lighting>,
}

fn load_soft() -> Option<Lighting> {
    let text = std::fs::read_to_string(soft_path()?).ok()?;
    toml::from_str::<SoftFile>(&text)
        .ok()?
        .lighting
        .filter(|l| matches!(l, Lighting::Gradient { .. }))
}

fn save_soft(l: Option<&Lighting>) {
    let Some(path) = soft_path() else { return };
    let file = SoftFile {
        lighting: l.cloned(),
    };
    match toml::to_string_pretty(&file) {
        Ok(text) => {
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("mouse: {}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("mouse: {e}"),
    }
}

/// Is G HUB running? Only its processes that talk to devices count (its
/// updater service runs all the time and never touches them).
struct GhubWatch {
    sys: sysinfo::System,
}

impl GhubWatch {
    fn new() -> Self {
        Self {
            sys: sysinfo::System::new(),
        }
    }

    fn running(&mut self) -> bool {
        self.sys
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        self.sys.processes().values().any(|p| {
            let n = p.name().to_string_lossy().to_lowercase();
            matches!(
                n.as_str(),
                "lghub.exe" | "lghub_agent.exe" | "lghub_system_tray.exe" | "lghub"
            )
        })
    }
}
