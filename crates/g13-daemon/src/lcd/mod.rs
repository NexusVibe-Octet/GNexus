//! LCD applets: what the G13's 160×43 screen shows.
//!
//! ```text
//!  engine thread ── LcdManager::tick (every engine tick, renders at ≤ 4 Hz)
//!                     ├─ BD (round button): next applet
//!                     │    Profil → Profils → Média → Horloge → Système
//!                     ├─ L1–L4: keys of the current applet (labels at the bottom)
//!                     ├─ banner (1.5 s) on applet / profile / menu change
//!                     └─ Engine::draw_lcd ─▶ DeviceCommand::DrawLcd + UiEvent::LcdFrame
//!  thread "g13-media" ── MediaProvider: now playing (1×/s while the media
//!                        applet is shown) + transport commands from L1–L3
//! ```
//!
//! Like Logitech Gaming Software. Frames are only sent when they change.

pub mod applets;
pub mod i18n;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use g13_core::{G13Key, G13LcdBuffer, LCD_WIDTH};
use sysinfo::{MemoryRefreshKind, System};

use crate::engine::{Engine, UiEvent};
use crate::platform::{self, MediaCommand, MediaInfo, PlaybackStatus};
use crate::profiles::ProfileManager;

/// Render period (clock seconds, scrolling titles).
pub const FRAME: Duration = Duration::from_millis(250);
/// How long the "profile / menu changed" banner stays.
pub const BANNER: Duration = Duration::from_millis(1500);

/// One entry of the profile list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileItem {
    pub id: String,
    pub name: String,
}

/// Everything an applet may show.
pub struct LcdContext<'a> {
    pub profile_name: &'a str,
    pub profile_default: bool,
    pub bank: &'static str,
    /// Executable in the foreground (the game), if known.
    pub game: Option<&'a str>,
    /// `game` is associated with the active profile (it selected it).
    pub game_matched: bool,
    /// Profile pinned everywhere (LCD choice or "Tester partout"), if any.
    pub pinned: Option<&'a str>,
    pub active_id: &'a str,
    /// All profiles, in display order.
    pub profiles: Vec<ProfileItem>,
    pub now: DateTime<Local>,
    /// Now playing, with the position extrapolated while playing.
    pub media: Option<MediaInfo>,
    pub sys: SysStats,
    /// Mouse battery (percent, charging) while a mouse is connected.
    pub mouse_battery: Option<(u8, bool)>,
    /// Increases at every rendered frame (scrolling).
    pub frame: u64,
    /// Language of the texts (that of the GUI).
    pub lang: i18n::Lang,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SysStats {
    /// 0–100.
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
}

/// What an applet asks the manager to do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LcdAction {
    None,
    /// Pin this profile everywhere, or `None` = automatic switching.
    Pin(Option<String>),
    Media(MediaCommand),
    /// Show another applet (index in the cycle).
    Goto(usize),
}

pub trait Applet: Send {
    /// Name shown in the banner when the applet is selected.
    fn title(&self) -> &'static str;
    /// Labels of L1–L4 drawn at the bottom of the screen ("" = unused).
    fn keys(&self) -> [&'static str; 4] {
        [""; 4]
    }
    /// The applet became visible.
    fn on_show(&mut self, _ctx: &LcdContext<'_>) {}
    /// L1–L4 (`index` 0–3) pressed while the applet is shown.
    fn on_key(&mut self, _index: usize, _ctx: &LcdContext<'_>) -> LcdAction {
        LcdAction::None
    }
    /// Needs "now playing" data (the media thread only works for it).
    fn wants_media(&self) -> bool {
        false
    }
    fn render(&mut self, lcd: &mut G13LcdBuffer, ctx: &LcdContext<'_>);
}

/// Shared slot filled by the media thread: info + when it was read.
type MediaSlot = Arc<Mutex<Option<(MediaInfo, Instant)>>>;

pub struct LcdManager {
    applets: Vec<Box<dyn Applet>>,
    current: usize,
    events: Receiver<UiEvent>,
    last: Option<G13LcdBuffer>,
    next: Instant,
    frame: u64,
    banner: Option<(String, Instant)>,
    media: MediaSlot,
    media_wanted: Arc<AtomicBool>,
    media_cmd: Sender<MediaCommand>,
    sys: System,
    sys_stats: SysStats,
    sys_next: Instant,
    mouse_battery: Option<(u8, bool)>,
    lang: i18n::LangCell,
}

impl LcdManager {
    /// `events` must be a fresh [`Engine::subscribe`] receiver.
    pub fn new(events: Receiver<UiEvent>) -> Self {
        let media: MediaSlot = Arc::new(Mutex::new(None));
        let media_wanted = Arc::new(AtomicBool::new(false));
        let (media_cmd, cmd_rx) = channel();
        spawn_media_thread(media.clone(), media_wanted.clone(), cmd_rx);
        let now = Instant::now();
        Self {
            applets: applets::all(),
            current: 0,
            events,
            last: None,
            next: now,
            frame: 0,
            banner: None,
            media,
            media_wanted,
            media_cmd,
            sys: System::new(),
            sys_stats: SysStats::default(),
            sys_next: now,
            mouse_battery: None,
            lang: i18n::LangCell::default(),
        }
    }

    /// Language of the LCD texts; set from any thread, used from the next frame.
    pub fn language(&self) -> i18n::LangCell {
        self.lang.clone()
    }

    pub fn current_applet(&self) -> usize {
        self.current
    }

    fn context<'a>(&self, engine: &Engine, profiles: &'a ProfileManager) -> LcdContext<'a> {
        let active = profiles.active_id();
        let store = profiles.store();
        let entry = store.get(active);
        let game = profiles.foreground_exe();
        let game_matched = match (entry, game) {
            (Some(e), Some(g)) => {
                let app = platform::ForegroundApp { process_name: g.to_string(), ..Default::default() };
                e.doc.rules.iter().any(|r| r.matches(&app))
            }
            _ => false,
        };
        LcdContext {
            profile_name: entry.map_or(active, |e| e.doc.profile.name.as_str()),
            profile_default: entry.is_some_and(|e| e.doc.is_default),
            bank: engine.active_bank_name(),
            game,
            game_matched,
            pinned: profiles.pinned(),
            active_id: active,
            profiles: store
                .ordered()
                .into_iter()
                .map(|e| ProfileItem { id: e.id.clone(), name: e.doc.profile.name.clone() })
                .collect(),
            now: Local::now(),
            media: current_media(&self.media),
            sys: self.sys_stats,
            mouse_battery: self.mouse_battery,
            frame: self.frame,
            lang: self.lang.get(),
        }
    }

    fn show(&mut self, index: usize, ctx: &LcdContext<'_>) {
        self.current = index % self.applets.len();
        self.applets[self.current].on_show(ctx);
        let title = i18n::tr(ctx.lang, self.applets[self.current].title());
        self.banner = Some((title.to_uppercase(), Instant::now()));
    }

    /// Call on the engine thread at every tick.
    pub fn tick(&mut self, engine: &mut Engine, profiles: &mut ProfileManager) {
        let mut redraw = false;
        let mut actions = Vec::new();
        let events: Vec<UiEvent> = self.events.try_iter().collect();
        if !events.is_empty() {
            let ctx = self.context(engine, profiles);
            for ev in events {
                match ev {
                    UiEvent::LcdButton { key: G13Key::Light, pressed: true } => {
                        self.show(self.current + 1, &ctx);
                        redraw = true;
                    }
                    UiEvent::LcdButton { key, pressed: true } => {
                        let i = match key {
                            G13Key::L1 => 0,
                            G13Key::L2 => 1,
                            G13Key::L3 => 2,
                            _ => 3,
                        };
                        actions.push(self.applets[self.current].on_key(i, &ctx));
                        redraw = true;
                    }
                    UiEvent::BankChanged(b) => {
                        self.banner = Some((format!("MENU {b:?}"), Instant::now()));
                        redraw = true;
                    }
                    UiEvent::ActiveProfile(id) => {
                        let name = profiles.store().profile(&id).map_or(id.clone(), |p| p.name.clone());
                        self.banner = Some((name.to_uppercase(), Instant::now()));
                        redraw = true;
                    }
                    UiEvent::Mouse(st) => {
                        let b = st.connected.then_some(st.battery).flatten().map(|p| (p, st.charging));
                        redraw |= b != self.mouse_battery;
                        self.mouse_battery = b;
                    }
                    // re-send the image after a (re)connection
                    UiEvent::Connected(true) => {
                        self.last = None;
                        redraw = true;
                    }
                    _ => {}
                }
            }
        }
        for action in actions {
            match action {
                LcdAction::None => {}
                LcdAction::Pin(id) => {
                    if id.is_none() {
                        self.banner = Some(("AUTO".into(), Instant::now()));
                    }
                    profiles.set_pinned(id.clone());
                    engine.emit(UiEvent::Pinned(id));
                }
                LcdAction::Media(cmd) => {
                    let _ = self.media_cmd.send(cmd);
                }
                LcdAction::Goto(i) => {
                    let ctx = self.context(engine, profiles);
                    self.show(i, &ctx);
                }
            }
        }
        // the media thread only works while its applet is on screen
        self.media_wanted.store(self.applets[self.current].wants_media(), Ordering::Relaxed);

        let now = Instant::now();
        if !redraw && now < self.next {
            return;
        }
        self.next = now + FRAME;
        self.frame += 1;
        if self.applets[self.current].title() == "Système" && now >= self.sys_next {
            self.sys_next = now + Duration::from_secs(1);
            self.sys.refresh_cpu_usage();
            self.sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
            self.sys_stats = SysStats {
                cpu: self.sys.global_cpu_usage(),
                mem_used: self.sys.used_memory(),
                mem_total: self.sys.total_memory(),
            };
        }

        let ctx = self.context(engine, profiles);
        let mut lcd = G13LcdBuffer::new();
        let applet = &mut self.applets[self.current];
        applet.render(&mut lcd, &ctx);
        applets::softkeys(&mut lcd, applet.keys().map(|k| i18n::tr(ctx.lang, k)));
        if let Some((text, since)) = &self.banner {
            if since.elapsed() < BANNER {
                draw_banner(&mut lcd, text);
            } else {
                self.banner = None;
            }
        }
        if self.last.as_ref() != Some(&lcd) {
            engine.draw_lcd(&lcd);
            self.last = Some(lcd);
        }
    }
}

/// Inverted box across the middle of the screen.
fn draw_banner(lcd: &mut G13LcdBuffer, text: &str) {
    let scale = if G13LcdBuffer::text_width(text, 2) <= LCD_WIDTH - 8 { 2 } else { 1 };
    let h = 7 * scale + 8;
    let y = (43 - h) / 2;
    lcd.fill_rect(0, y, LCD_WIDTH, h, false);
    let w = G13LcdBuffer::text_width(text, scale).min(LCD_WIDTH - 4);
    lcd.draw_text_scaled((LCD_WIDTH - w) / 2, y + 4, text, scale);
    lcd.invert_rect(0, y, LCD_WIDTH, h);
}

/// Latest media info, its position moved forward while playing.
fn current_media(slot: &MediaSlot) -> Option<MediaInfo> {
    let (mut info, read_at) = slot.lock().unwrap().clone()?;
    if info.status == PlaybackStatus::Playing
        && let Some(p) = info.position_s.as_mut()
    {
        *p += read_at.elapsed().as_secs_f64();
        if let Some(d) = info.duration_s {
            *p = p.min(d);
        }
    }
    Some(info)
}

fn spawn_media_thread(slot: MediaSlot, wanted: Arc<AtomicBool>, commands: Receiver<MediaCommand>) {
    let r = thread::Builder::new().name("g13-media".into()).spawn(move || {
        let mut provider = platform::default_media_provider();
        loop {
            if wanted.load(Ordering::Relaxed) {
                let info = match provider.now_playing() {
                    Ok(i) => i,
                    Err(e) => {
                        log::debug!("now playing: {e}");
                        None
                    }
                };
                *slot.lock().unwrap() = info.map(|i| (i, Instant::now()));
            }
            // wait a second, or less when L1–L3 sends a command
            match commands.recv_timeout(Duration::from_secs(1)) {
                Ok(cmd) => {
                    if let Err(e) = provider.send(cmd) {
                        log::warn!("media command: {e}");
                    }
                    thread::sleep(Duration::from_millis(250)); // let the player update
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    if let Err(e) = r {
        log::warn!("no media thread: {e}");
    }
}
