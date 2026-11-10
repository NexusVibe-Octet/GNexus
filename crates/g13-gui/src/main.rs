//! G Nexus — configuration app for the Logitech G13.
//!
//! One process, like Logitech Gaming Software: the window (Tauri, HTML/CSS
//! front-end in `ui/`), a notification-area icon, and the g13-daemon
//! [`Runtime`] (HID + engine threads) which keeps driving the G13 while the
//! window is hidden. The window edits the TOML profiles; the engine applies
//! them through its hot reload.
//!
//! ```text
//! g13-studio [--dry-run] [--poll] [--autostart]
//!   --autostart   passed by the "start with Windows" entry: honours "start minimized"
//! ```
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod conflicts;
mod i18n;
mod library;
mod profiles_io;
mod settings;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use g13_daemon::engine::UiEvent;
use g13_daemon::headset::{HeadsetCommand, HeadsetStatus};
use g13_daemon::mouse::{Lighting, MouseCommand, MouseSettings, MouseStatus, SoftProfile};
use g13_core::G13Key;
use g13_daemon::profiles::FileDto;
use g13_daemon::runtime::{Request, Runtime, RuntimeOptions};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use settings::Settings;

/// Live device state, kept up to date from the engine's [`UiEvent`]s.
#[derive(Debug, Clone, Default, Serialize)]
struct Status {
    connected: bool,
    bank: String,
    active_profile: String,
    recording: bool,
    display_on: bool,
    dry_run: bool,
    /// Why the engine is not running, if it failed to start.
    error: Option<String>,
    /// Last image shown on the G13's LCD (960 bytes, 6 pages × 160 columns).
    lcd: Option<Vec<u8>>,
    /// Profile pinned everywhere ("Tester partout"), if any.
    pinned: Option<String>,
    /// Last known state of the mouse (G502 X PLUS…).
    mouse: Option<MouseStatus>,
    /// Last known state of the headset (PRO X Wireless…).
    headset: Option<HeadsetStatus>,
}

/// Event pushed to the front-end (`g13` channel).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Ev {
    Key { key: String, pressed: bool },
    Bank { bank: String },
    Profile { id: String },
    Connected { value: bool },
    Record { value: bool },
    Display { value: bool },
    /// LCD image, same layout as `Status::lcd`.
    Lcd { frame: Vec<u8> },
    /// Profile pinned from the G13's LCD (`None` = automatic).
    Pinned { id: Option<String> },
    /// The virtual gamepad failed (ViGEmBus missing…).
    GamepadError { message: String },
    /// New state of the mouse.
    Mouse { status: Box<MouseStatus> },
    /// New state of the headset.
    Headset { status: Box<HeadsetStatus> },
}

struct AppState {
    runtime: Mutex<Option<Runtime>>,
    profiles_dir: Option<PathBuf>,
    status: Arc<Mutex<Status>>,
    settings: Mutex<Settings>,
    settings_path: Option<PathBuf>,
    /// Language the page is shown in (tray menu, file dialogs).
    ui_lang: Mutex<String>,
    /// Macro recording in progress (RECORD button).
    recorder: Mutex<Option<Box<dyn g13_daemon::platform::InputRecorder>>>,
    /// What the overlay shows once its page has loaded (it is created when called).
    overlay_pending: Mutex<Option<OverlayOpen>>,
}

impl AppState {
    fn dir(&self) -> Result<&PathBuf, String> {
        self.profiles_dir.as_ref().ok_or_else(|| "aucun dossier de profils".to_string())
    }

    fn request(&self, req: Request) {
        if let Some(rt) = self.runtime.lock().unwrap().as_ref() {
            rt.request(req);
        }
    }

    fn rescan(&self) {
        self.request(Request::RescanProfiles);
    }

    /// `text` (French) in the interface language.
    fn tr(&self, text: &'static str) -> &'static str {
        i18n::tr(&self.ui_lang.lock().unwrap(), text)
    }
}

fn parse_overlay_key(id: Option<&str>) -> Option<G13Key> {
    id.and_then(G13Key::from_id).filter(|k| k.is_g_key())
}

// ---- commands ------------------------------------------------------------------

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
    state.status.lock().unwrap().clone()
}

/// Last known state of the mouse (`None` until the mouse thread reported).
#[tauri::command]
fn get_mouse(state: State<AppState>) -> Option<MouseStatus> {
    state.status.lock().unwrap().mouse.clone()
}

/// Write these settings into the mouse's active onboard profile. The result
/// arrives as a `mouse` event (message / error).
#[tauri::command]
fn apply_mouse(state: State<AppState>, settings: MouseSettings, sector: Option<u16>) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    let rt = rt.as_ref().ok_or("moteur arrêté")?;
    rt.mouse(MouseCommand::Apply { sector, settings });
    Ok(())
}

/// Save mouse settings (the profile shown) to a `.g13mouse` file (JSON). Returns the path.
#[tauri::command]
async fn export_mouse_profile(app: AppHandle, state: State<'_, AppState>, settings: MouseSettings) -> Result<Option<String>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Exporter un profil de souris"))
        .set_file_name("G502 X PLUS.g13mouse")
        .add_filter(state.tr("Profil de souris G Nexus"), &["g13mouse"])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// Read a `.g13mouse` file (checked by deserialising it).
#[tauri::command]
async fn import_mouse_profile(app: AppHandle, state: State<'_, AppState>) -> Result<Option<MouseSettings>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Importer un profil de souris"))
        .add_filter(state.tr("Profil de souris G Nexus"), &["g13mouse"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map(Some).map_err(|e| format!("fichier de profil de souris invalide ({e})"))
}

/// Make an onboard profile of the mouse the active one.
#[tauri::command]
fn select_mouse_profile(state: State<AppState>, sector: u16) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::Select(sector));
    Ok(())
}

/// Last known state of the headset.
#[tauri::command]
fn get_headset(state: State<AppState>) -> Option<HeadsetStatus> {
    state.status.lock().unwrap().headset.clone()
}

/// Sidetone (mic heard in the headset), 0..100.
#[tauri::command]
fn set_headset_sidetone(state: State<AppState>, level: u8) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(HeadsetCommand::SetSidetone(level));
    Ok(())
}

/// Automatic power-off delay in minutes (0 = never).
#[tauri::command]
fn set_headset_sleep(state: State<AppState>, minutes: u8) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(HeadsetCommand::SetSleep(minutes));
    Ok(())
}

/// An equalizer profile in a file (`.gneq`, JSON).
#[derive(serde::Serialize, serde::Deserialize)]
struct EqFile {
    format: String,
    version: u32,
    profile: g13_daemon::headset::eq::EqProfile,
}
const EQ_FORMAT: &str = "gnexus-eq";

/// Save an equalizer profile to a file. Returns the path.
#[tauri::command]
async fn export_eq_profile(app: AppHandle, state: State<'_, AppState>, profile: g13_daemon::headset::eq::EqProfile) -> Result<Option<String>, String> {
    let name = if profile.name.trim().is_empty() { "egaliseur".to_string() } else { profile.name.clone() };
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Exporter un profil d'égaliseur"))
        .set_file_name(format!("{name}.gneq"))
        .add_filter(state.tr("Profil d'égaliseur"), &["gneq"])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&EqFile { format: EQ_FORMAT.into(), version: 1, profile }).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// Read an equalizer profile file (checked: 10 bands, gains within ±12 dB).
#[tauri::command]
async fn import_eq_profile(app: AppHandle, state: State<'_, AppState>) -> Result<Option<g13_daemon::headset::eq::EqProfile>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Importer un profil d'égaliseur"))
        .add_filter(state.tr("Profil d'égaliseur"), &["gneq"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let f: EqFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("fichier de profil d'égaliseur invalide ({e})"))?;
    if f.format != EQ_FORMAT || f.profile.gains.len() != g13_dsp::DEFAULT_FREQS.len() {
        return Err("fichier de profil d'égaliseur invalide".into());
    }
    let mut p = f.profile;
    for g in &mut p.gains {
        *g = (*g).clamp(-12, 12);
    }
    Ok(Some(p))
}

/// Send a command to the headset thread.
fn headset_cmd(state: &State<AppState>, cmd: HeadsetCommand) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(cmd);
    Ok(())
}

/// Save equalizer profile `index` (`None`: a new one); it is heard at once.
#[tauri::command]
fn save_eq_profile(state: State<AppState>, index: Option<usize>, profile: g13_daemon::headset::eq::EqProfile) -> Result<(), String> {
    headset_cmd(&state, HeadsetCommand::SaveEqProfile { index, profile })
}

#[tauri::command]
fn delete_eq_profile(state: State<AppState>, index: usize) -> Result<(), String> {
    headset_cmd(&state, HeadsetCommand::DeleteEqProfile(index))
}

/// Hear this equalizer profile now (until the foreground application changes).
#[tauri::command]
fn use_eq_profile(state: State<AppState>, index: usize) -> Result<(), String> {
    headset_cmd(&state, HeadsetCommand::UseEqProfile(index))
}

#[tauri::command]
fn set_eq_default(state: State<AppState>, index: usize) -> Result<(), String> {
    headset_cmd(&state, HeadsetCommand::SetEqDefault(index))
}

/// "Tester partout" for an equalizer profile (`None`: stop).
#[tauri::command]
fn pin_eq_profile(state: State<AppState>, index: Option<usize>) -> Result<(), String> {
    headset_cmd(&state, HeadsetCommand::PinEqProfile(index))
}

/// Windows volume of the headset's output, 0..100.
#[tauri::command]
fn set_headset_volume(state: State<AppState>, volume: u8) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(HeadsetCommand::SetVolume(volume));
    Ok(())
}

#[tauri::command]
fn set_headset_mute(state: State<AppState>, mute: bool) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(HeadsetCommand::SetMute(mute));
    Ok(())
}

/// Gains of the headset's built-in equalizer (one per band, dB).
#[tauri::command]
fn set_headset_eq(state: State<AppState>, gains: Vec<i8>) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.headset(HeadsetCommand::SetEqualizer(gains));
    Ok(())
}

/// Enable / disable an onboard profile of the mouse.
#[tauri::command]
fn enable_mouse_profile(state: State<AppState>, sector: u16, enabled: bool) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::SetEnabled { sector, enabled });
    Ok(())
}

/// The macro library (every macro of the G13 and the mouse, to reuse anywhere).
#[tauri::command]
fn macro_library(state: State<AppState>) -> Result<library::Library, String> {
    Ok(library::load(state.dir()?))
}

/// Put a macro in the library (`old`: its previous name, after a rename).
#[tauri::command]
fn save_library_macro(
    state: State<AppState>,
    name: String,
    mac: g13_daemon::profiles::format::MacroDto,
    old: Option<String>,
) -> Result<(), String> {
    let dir = state.dir()?;
    let mut lib = library::load(dir);
    if let Some(old) = old.filter(|o| *o != name) {
        lib.remove(&old);
    }
    lib.insert(name, mac);
    library::save(dir, &lib)
}

/// Take a macro out of the library (the profiles that use it keep their copy).
#[tauri::command]
fn delete_library_macro(state: State<AppState>, name: String) -> Result<(), String> {
    let dir = state.dir()?;
    let mut lib = library::load(dir);
    lib.remove(&name);
    library::save(dir, &lib)
}

/// G Nexus mouse profile used where no application has its own.
#[tauri::command]
fn set_mouse_soft_default(state: State<AppState>, index: usize) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::SetSoftDefault(index));
    Ok(())
}

/// "Tester partout" for a mouse profile (`None`: stop the test).
#[tauri::command]
fn pin_mouse_soft(state: State<AppState>, index: Option<usize>) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::PinSoft(index));
    Ok(())
}

/// RECORD: start capturing the keyboard and mouse buttons for a macro.
#[tauri::command]
fn record_start(state: State<AppState>) -> Result<(), String> {
    let mut r = state.recorder.lock().unwrap();
    if let Some(old) = r.take() {
        old.stop();
    }
    *r = Some(g13_daemon::platform::start_recording().map_err(|e| e.detail())?);
    Ok(())
}

/// STOP: the capture as macro steps (`delays`: keep the real delays). The click on
/// STOP is left out.
#[tauri::command]
fn record_stop(state: State<AppState>, delays: bool) -> Result<Vec<String>, String> {
    let Some(r) = state.recorder.lock().unwrap().take() else { return Ok(Vec::new()) };
    let events = r.stop();
    Ok(g13_daemon::recording::to_steps(&events, g13_daemon::recording::StepOptions { delays, drop_stop_click: true }))
}

/// Onboard profiles off (`on`: G Nexus drives the mouse in real time) or back on.
#[tauri::command]
fn set_mouse_host(state: State<AppState>, on: bool) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::SetHost(on));
    Ok(())
}

/// Save a G Nexus mouse profile (`index` = None: a new one).
#[tauri::command]
fn save_mouse_soft(state: State<AppState>, index: Option<usize>, profile: SoftProfile) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::SaveSoft { index, profile });
    Ok(())
}

#[tauri::command]
fn delete_mouse_soft(state: State<AppState>, index: usize) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::DeleteSoft(index));
    Ok(())
}

/// Show this lighting (and DPI) on the mouse now, without saving it.
#[tauri::command]
fn preview_mouse(state: State<AppState>, lighting: Lighting, dpi: Option<u16>) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::Preview { lighting, dpi });
    Ok(())
}

/// Drop the previewed changes: the mouse shows its saved profile again.
#[tauri::command]
fn revert_mouse(state: State<AppState>) -> Result<(), String> {
    let rt = state.runtime.lock().unwrap();
    rt.as_ref().ok_or("moteur arrêté")?.mouse(MouseCommand::Revert);
    Ok(())
}

#[tauri::command]
fn list_profiles(state: State<AppState>) -> Result<Vec<profiles_io::ProfileSummary>, String> {
    Ok(profiles_io::list(state.dir()?))
}

#[tauri::command]
fn load_profile(state: State<AppState>, id: String) -> Result<FileDto, String> {
    profiles_io::load(state.dir()?, &id)
}

#[tauri::command]
fn save_profile(state: State<AppState>, id: String, profile: FileDto) -> Result<(), String> {
    profiles_io::save(state.dir()?, &id, &profile)?;
    state.rescan();
    Ok(())
}

#[tauri::command]
fn create_profile(state: State<AppState>, name: String, from: Option<String>) -> Result<String, String> {
    let id = profiles_io::create(state.dir()?, &name, from.as_deref())?;
    state.rescan();
    Ok(id)
}

#[tauri::command]
fn delete_profile(state: State<AppState>, id: String) -> Result<(), String> {
    profiles_io::delete(state.dir()?, &id)?;
    state.rescan();
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct SettingsView {
    autostart: bool,
    start_minimized: bool,
    close_to_tray: bool,
    overlay_key: Option<String>,
    overlay_position: String,
    language: Option<String>,
    keyboard_layout: Option<String>,
    profiles_dir: String,
    auto_backup: String,
    auto_backup_keep: u32,
    /// Date of the newest scheduled backup.
    last_backup: Option<String>,
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<AppState>) -> SettingsView {
    let s = state.settings.lock().unwrap().clone();
    SettingsView {
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        start_minimized: s.start_minimized,
        close_to_tray: s.close_to_tray,
        overlay_key: s.overlay_key,
        overlay_position: s.overlay_position,
        language: s.language,
        keyboard_layout: s.keyboard_layout,
        profiles_dir: state.profiles_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        auto_backup: s.auto_backup,
        auto_backup_keep: s.auto_backup_keep,
        last_backup: state.profiles_dir.as_deref().and_then(backup::last_auto),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // one argument per preference, as the page sends them
fn set_settings(
    app: AppHandle,
    state: State<AppState>,
    autostart: bool,
    start_minimized: bool,
    close_to_tray: bool,
    overlay_key: Option<String>,
    overlay_position: String,
    auto_backup: String,
    auto_backup_keep: u32,
) -> Result<(), String> {
    let launcher = app.autolaunch();
    if launcher.is_enabled().unwrap_or(false) != autostart {
        let r = if autostart { enable_autostart(&app) } else { launcher.disable().map_err(|e| e.to_string()) };
        r.map_err(|e| format!("démarrage automatique : {e}"))?;
    }
    let key = parse_overlay_key(overlay_key.as_deref());
    state.request(Request::SetOverlayKey(key));
    let mut s = state.settings.lock().unwrap();
    s.start_minimized = start_minimized;
    s.close_to_tray = close_to_tray;
    s.overlay_key = key.map(|k| k.id().to_string());
    s.overlay_position = match overlay_position.as_str() {
        "left" | "center" => overlay_position,
        _ => "right".into(),
    };
    s.auto_backup = if backup::period_days(&auto_backup).is_some() { auto_backup } else { "off".into() };
    s.auto_backup_keep = auto_backup_keep.clamp(1, 100);
    s.save(state.settings_path.as_deref())
}

/// A backup of the whole configuration now, in the scheduled backups. Returns its date.
#[tauri::command]
fn backup_now(app: AppHandle, state: State<AppState>) -> Result<Option<String>, String> {
    let settings = state.settings.lock().unwrap().clone();
    backup::auto_backup(state.dir()?, &settings, app.autolaunch().is_enabled().unwrap_or(false), true)?;
    Ok(backup::last_auto(state.dir()?))
}

/// Scheduled backups: checked a minute after start, then every hour.
fn spawn_auto_backup(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        loop {
            let state = app.state::<AppState>();
            if let Ok(dir) = state.dir() {
                let settings = state.settings.lock().unwrap().clone();
                match backup::auto_backup(dir, &settings, app.autolaunch().is_enabled().unwrap_or(false), false) {
                    Ok(Some(p)) => log::info!("scheduled backup: {}", p.display()),
                    Ok(None) => {}
                    Err(e) => log::warn!("scheduled backup failed: {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    });
}

/// Interface language (`None` = follow the system) and picker keyboard
/// (`None` = the language's). Both windows reload in the new language.
#[tauri::command]
fn set_language(
    app: AppHandle,
    state: State<AppState>,
    language: Option<String>,
    keyboard_layout: Option<String>,
) -> Result<(), String> {
    {
        let mut s = state.settings.lock().unwrap();
        s.language = language.filter(|l| i18n::LANGS.contains(&l.as_str()));
        s.keyboard_layout = keyboard_layout.filter(|l| ["azerty", "us", "qwertz", "it", "es"].contains(&l.as_str()));
        s.save(state.settings_path.as_deref())?;
    }
    let _ = app.emit("language-changed", ());
    Ok(())
}

/// The page reports the language it is shown in: the tray menu follows.
#[tauri::command]
fn ui_language(app: AppHandle, state: State<AppState>, lang: String) -> Result<(), String> {
    if !i18n::LANGS.contains(&lang.as_str()) {
        return Ok(());
    }
    *state.ui_lang.lock().unwrap() = lang.clone();
    // the G13 screen speaks the same language
    if let Some(rt) = state.runtime.lock().unwrap().as_ref() {
        rt.set_lcd_language(&lang);
    }
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(&app, &lang).map_err(|e| e.to_string())?)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn tray_menu(app: &AppHandle, lang: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", i18n::tr(lang, "Ouvrir G Nexus"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", i18n::tr(lang, "Quitter"), true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
}

// ---- whole configuration: export / import / reset ----------------------------------

/// Apply restored preferences: file, overlay key, autostart.
fn apply_settings(app: &AppHandle, state: &AppState, settings: Settings, autostart: Option<bool>) -> Result<(), String> {
    state.request(Request::SetOverlayKey(parse_overlay_key(settings.overlay_key.as_deref())));
    settings.save(state.settings_path.as_deref())?;
    *state.settings.lock().unwrap() = settings;
    if let Some(on) = autostart {
        let launcher = app.autolaunch();
        if launcher.is_enabled().unwrap_or(false) != on {
            let r = if on { enable_autostart(app) } else { launcher.disable().map_err(|e| e.to_string()) };
            r.map_err(|e| format!("démarrage automatique : {e}"))?;
        }
    }
    Ok(())
}

/// Save the current configuration to `<config>/backups/` (before an import / reset).
fn safety_backup(app: &AppHandle, state: &AppState, label: &str) -> Result<(), String> {
    let dir = state.dir()?;
    let b = backup::collect(dir, &state.settings.lock().unwrap(), app.autolaunch().is_enabled().unwrap_or(false))?;
    let path = backup::auto_path(dir, label).ok_or("aucun dossier de configuration")?;
    backup::write(&path, &b)?;
    log::info!("configuration saved to {}", path.display());
    Ok(())
}

/// Every profile + the preferences in one file. Returns the file written (`None` = cancelled).
#[tauri::command]
async fn export_config(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let b = backup::collect(state.dir()?, &state.settings.lock().unwrap().clone(), app.autolaunch().is_enabled().unwrap_or(false))?;
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Exporter la configuration de G Nexus"))
        .set_file_name(format!("G Nexus {}.{}", chrono::Local::now().format("%Y-%m-%d"), backup::EXTENSION))
        .add_filter(state.tr("Sauvegarde G Nexus"), &[backup::EXTENSION])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    backup::write(&path, &b)?;
    Ok(Some(path.display().to_string()))
}

/// Replace every profile and the preferences by a backup. Returns the number of profiles.
#[tauri::command]
async fn import_config(app: AppHandle, state: State<'_, AppState>) -> Result<Option<usize>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Importer une configuration G Nexus"))
        .add_filter(state.tr("Sauvegarde G Nexus"), &[backup::EXTENSION])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let b = backup::parse(&text)?;
    safety_backup(&app, &state, "avant-import")?;
    backup::restore_profiles(state.dir()?, &b)?;
    backup::restore_files(state.dir()?, &b)?;
    apply_settings(&app, &state, b.settings.clone(), Some(b.autostart))?;
    state.rescan();
    log::info!("configuration imported from {} ({} profiles)", path.display(), b.profiles.len());
    Ok(Some(b.profiles.len()))
}

/// Back to a fresh install: demo profile only, default preferences
/// ("start with Windows" is left as it is). A backup is saved first.
#[tauri::command]
fn reset_config(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    safety_backup(&app, &state, "avant-reinitialisation")?;
    backup::reset_profiles(state.dir()?)?;
    let language = state.settings.lock().unwrap().language.clone();
    // the language just chosen stays: resetting should not switch the UI language under the user
    apply_settings(&app, &state, Settings { language, ..Settings::default() }, None)?;
    state.request(Request::Pin(None));
    state.status.lock().unwrap().pinned = None;
    state.rescan();
    log::info!("configuration reset");
    Ok(())
}

/// Open the folder holding the automatic backups.
#[tauri::command]
fn open_backups_dir(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let dir = state.dir()?.parent().map(|p| p.join("backups")).ok_or("aucun dossier")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// Profile shown in the window: applied to the G13 while the window has the focus.
#[tauri::command]
fn set_preview(state: State<AppState>, id: Option<String>) {
    state.request(Request::Preview(id));
}

/// "Tester partout": keep profile `id` active in every application (`None` to stop).
#[tauri::command]
fn set_pinned(state: State<AppState>, id: Option<String>) {
    state.status.lock().unwrap().pinned = id.clone();
    state.request(Request::Pin(id));
}

// ---- in-game overlay -------------------------------------------------------------

/// Payload sent to the overlay page when it opens: what the G13 is using.
#[derive(Debug, Clone, Serialize)]
struct OverlayOpen {
    profile: String,
    bank: String,
    /// Executable in the foreground when the overlay was called (the game),
    /// `None` if it is G Nexus itself or unknown.
    game: Option<String>,
}

/// The application currently in the foreground, unless it is us.
fn foreground_game() -> Option<String> {
    let app = g13_daemon::platform::default_foreground_watcher().current().ok()??;
    let me = std::env::current_exe().ok()?.file_name()?.to_string_lossy().into_owned();
    (!app.process_name.is_empty() && !app.process_name.eq_ignore_ascii_case(&me)).then_some(app.process_name)
}

#[derive(Debug, Clone, Serialize)]
struct DetectedApp {
    exe: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
struct DetectedApps {
    /// Applications with a window right now.
    running: Vec<DetectedApp>,
    /// Applications recently in the foreground (closed games stay listed).
    recent: Vec<DetectedApp>,
}

/// Desktop helpers (Plasma shell, launcher, portals…) nobody binds a profile to.
#[cfg(not(windows))]
const NOT_APPS: &[&str] = &[
    "plasmashell",
    "kwin_wayland",
    "kwin_x11",
    "krunner",
    "xdg-desktop-portal-kde",
    "polkit-kde-authentication-agent-1",
    "ksmserver-logout-greeter",
];

/// Windows-shell helpers nobody binds a profile to.
#[cfg(windows)]
const NOT_APPS: &[&str] = &[
    "explorer.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "lockapp.exe",
    "systemsettings.exe",
];

/// Applications the user can bind a profile to, for the "Applications" tab.
#[tauri::command]
fn detected_apps(state: State<AppState>) -> DetectedApps {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let keep = |exe: &str| {
        let l = exe.to_lowercase();
        !l.is_empty() && l != me && !NOT_APPS.contains(&l.as_str())
    };
    let running: Vec<DetectedApp> = g13_daemon::platform::running_apps()
        .into_iter()
        .filter(|a| keep(&a.process_name))
        .map(|a| DetectedApp { exe: a.process_name, title: a.title })
        .collect();
    let recent = state
        .runtime
        .lock()
        .unwrap()
        .as_ref()
        .map(|rt| rt.recent_apps())
        .unwrap_or_default()
        .into_iter()
        .filter(|a| keep(&a.process_name) && !running.iter().any(|r| r.exe.eq_ignore_ascii_case(&a.process_name)))
        .map(|a| DetectedApp { exe: a.process_name, title: a.title })
        .collect();
    DetectedApps { running, recent }
}

/// Pick an executable with the file dialog; returns its file name (what the
/// association rules compare), or `None` if cancelled.
#[tauri::command]
async fn pick_exe(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Choisir l'exécutable du jeu ou de l'application"))
        .add_filter(state.tr("Programmes"), &["exe"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Overlay: make profile `id` the one used by `exe` (the game).
#[tauri::command]
fn assign_game(state: State<AppState>, id: String, exe: String) -> Result<(), String> {
    profiles_io::assign_app(state.dir()?, &id, &exe)?;
    state.rescan();
    Ok(())
}

/// Show the overlay on the profile / menu active on the G13 (the game's), or
/// hide it if already shown. Triggered by the overlay G-key.
fn toggle_overlay(app: &AppHandle) {
    let existing = app.get_webview_window("overlay");
    log::info!("overlay toggle (open: {})", existing.is_some());
    if existing.is_some() {
        hide_overlay(app);
        return;
    }
    let state = app.state::<AppState>();
    let open = {
        let s = state.status.lock().unwrap();
        OverlayOpen { profile: s.active_profile.clone(), bank: s.bank.clone(), game: None }
    };
    // read before our window takes the focus
    let open = OverlayOpen { game: foreground_game(), ..open };
    // Our window is about to take the focus: keep the game's profile on the
    // G13 instead of switching to whatever the main window shows.
    state.request(Request::Preview(Some(open.profile.clone())));
    *state.overlay_pending.lock().unwrap() = Some(open);
    // In-game overlay: borderless, transparent, always on top, outside the
    // taskbar. Created only now (its web view costs memory) and shown by its
    // page once loaded (`overlay_ready`). Works over windowed / borderless-
    // fullscreen games (an exclusive fullscreen game is minimised by Windows).
    // Wayland: a window can neither place itself nor stay above the game; the KWin script
    // of the foreground detection does it, for a window with this title (Plasma only).
    #[cfg(target_os = "linux")]
    let title = {
        g13_daemon::platform::linux::foreground::ensure_started();
        g13_daemon::platform::linux::foreground::overlay_title(&state.settings.lock().unwrap().overlay_position)
    };
    #[cfg(not(target_os = "linux"))]
    let title = "G Nexus — overlay";
    let built = tauri::WebviewWindowBuilder::new(app, "overlay", tauri::WebviewUrl::App("index.html#overlay".into()))
        .title(title)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .inner_size(OVERLAY_W, 860.0)
        .visible(false)
        .build();
    if let Err(e) = built {
        log::warn!("overlay window: {e}");
    }
}

/// The overlay page has loaded: what to show; the window is placed and shown.
#[tauri::command]
fn overlay_ready(app: AppHandle, state: State<AppState>) -> Option<OverlayOpen> {
    let open = state.overlay_pending.lock().unwrap().take();
    if let Some(w) = app.get_webview_window("overlay") {
        let position = state.settings.lock().unwrap().overlay_position.clone();
        place_overlay(&app, &w, &position);
        let _ = w.show();
        let _ = w.set_focus();
    }
    open
}

/// Overlay panel size (logical px) and margin from the screen edge.
const OVERLAY_W: f64 = 780.0;
const OVERLAY_MAX_H: f64 = 940.0;
const OVERLAY_MARGIN: f64 = 24.0;

/// Size the overlay panel to the screen under the mouse (the game's) and dock
/// it left / centre / right, vertically centred.
fn place_overlay(app: &AppHandle, w: &tauri::WebviewWindow, position: &str) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let (mx, my) = (m.position().x as f64, m.position().y as f64);
    let (mw, mh) = (m.size().width as f64, m.size().height as f64);
    let width = (OVERLAY_W * scale).min(mw - 2.0 * OVERLAY_MARGIN * scale);
    let height = (mh * 0.9).min(OVERLAY_MAX_H * scale);
    let margin = OVERLAY_MARGIN * scale;
    let x = match position {
        "left" => mx + margin,
        "center" => mx + (mw - width) / 2.0,
        _ => mx + mw - width - margin,
    };
    let y = my + (mh - height) / 2.0;
    let _ = w.set_size(tauri::PhysicalSize::new(width.round() as u32, height.round() as u32));
    let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// Close the overlay (its web view is freed); Windows gives the focus back to the game underneath.
fn hide_overlay(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.destroy();
    }
    // the main window re-sends its own preview and reloads what the overlay edited
    let _ = app.emit_to("main", "overlay-closed", ());
}

#[tauri::command]
fn close_overlay(app: AppHandle) {
    hide_overlay(&app);
}

#[derive(Debug, Clone, Serialize)]
struct ImportResult {
    id: String,
    name: String,
    /// Bindings that could not be converted (shown to the user).
    skipped: Vec<String>,
}

/// Pick a Logitech Gaming Software XML profile and add it as a new profile.
/// `async` so the blocking file dialog does not run on the main thread.
#[tauri::command]
async fn import_lgs(app: AppHandle, state: State<'_, AppState>) -> Result<Option<ImportResult>, String> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Importer un profil Logitech Gaming Software"))
        .add_filter(state.tr("Profil Logitech Gaming Software (XML)"), &["xml"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let imported = g13_daemon::profiles::lgs::import(text.trim_start_matches('\u{feff}')).map_err(|e| e.0)?;
    let id = profiles_io::create_from(state.dir()?, imported.dto.clone())?;
    state.rescan();
    log::info!("imported LGS profile {} as '{id}' ({} skipped)", path.display(), imported.skipped.len());
    Ok(Some(ImportResult { id, name: imported.dto.name, skipped: imported.skipped }))
}

/// Save profile `id` as a Logitech Gaming Software XML file. Returns the
/// bindings LGS cannot hold (`None` if the dialog was cancelled).
#[tauri::command]
async fn export_lgs(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Option<Vec<String>>, String> {
    let dto = profiles_io::load(state.dir()?, &id)?;
    let (xml, skipped) = g13_daemon::profiles::lgs::export(&dto);
    let Some(file) = app
        .dialog()
        .file()
        .set_title(state.tr("Exporter vers Logitech Gaming Software"))
        .set_file_name(format!("{}.xml", dto.name))
        .add_filter(state.tr("Profil Logitech Gaming Software (XML)"), &["xml"])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, xml).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(skipped))
}

/// Logitech software running alongside (it steals the G13's key reports).
#[tauri::command]
fn conflicting_software() -> Vec<conflicts::Conflict> {
    conflicts::running()
}

/// Stop the conflicting Logitech programs (explicit user action in the window).
#[tauri::command]
fn close_conflicting_software() -> Vec<conflicts::Conflict> {
    conflicts::close_all()
}

/// Can the stick act as a virtual Xbox 360 controller? Err = why not.
#[tauri::command]
fn gamepad_status() -> Result<(), String> {
    g13_daemon::platform::virtual_gamepad_available().map_err(|e| e.detail())
}

/// Register G Nexus to start with the session. Linux: the plugin writes the path unquoted in
/// `~/.config/autostart/G Nexus.desktop` (`Exec=/path/G Nexus_1.1.1_amd64.AppImage --autostart`):
/// a path with a space (the AppImage is named after "G Nexus") would not start. Quote it.
fn enable_autostart(app: &AppHandle) -> Result<(), String> {
    app.autolaunch().enable().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let file = std::path::Path::new(&home)
            .join(".config/autostart")
            .join(format!("{}.desktop", app.package_info().name));
        if let Ok(text) = std::fs::read_to_string(&file) {
            let fixed = quote_desktop_exec(&text);
            if fixed != text {
                std::fs::write(&file, fixed).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// `Exec=<path> --autostart` with `<path>` quoted (desktop entry rules) when it is not already.
#[cfg(target_os = "linux")]
fn quote_desktop_exec(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match line.strip_prefix("Exec=") {
            Some(cmd) if !cmd.starts_with('"') => {
                let (path, args) = match cmd.rfind(" --autostart") {
                    Some(i) => (&cmd[..i], &cmd[i..]),
                    None => (cmd.trim_end(), ""),
                };
                let q: String = path.chars().flat_map(|c| match c {
                    '"' | '`' | '$' | '\\' => vec!['\\', c],
                    c => vec![c],
                }).collect();
                out.push(format!("Exec=\"{q}\"{args}"));
            }
            _ => out.push(line.to_string()),
        }
    }
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Linux: can G Nexus open its devices and `/dev/uinput` (see `platform::linux::access`).
/// `None` on the other systems (nothing to install).
#[tauri::command]
fn device_access() -> Option<serde_json::Value> {
    #[cfg(target_os = "linux")]
    {
        serde_json::to_value(g13_daemon::platform::linux::access::status()).ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Linux: install G Nexus's udev rules (administrator password asked once by polkit).
/// Returns `true` when G Nexus must restart (its engine could not start without `/dev/uinput`).
#[tauri::command]
async fn install_device_access(record: bool) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(move || g13_daemon::platform::linux::access::install(record))
            .await
            .map_err(|e| e.to_string())?
            .map(|r| r.restart)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = record;
        Err("rien à installer sur ce système".into())
    }
}

/// Restart G Nexus (after the device access was granted).
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// Every key name the engine accepts (the front-end checks its catalogue against it).
#[tauri::command]
fn key_names() -> Vec<&'static str> {
    g13_daemon::keycode::KeyCode::ALL.iter().map(|k| k.name()).collect()
}

/// G Nexus's version (tauri.conf.json), for the "About" section.
#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// The author's pages (About section). Fixed here: the page only names one of them.
const LINKS: &[(&str, &str)] = &[
    ("github", "https://github.com/NexusVibe-Octet/gnexus"),
    ("kofi", "https://ko-fi.com/W4F728ZFSH"),
    ("jaspmakes", "https://www.youtube.com/watch?v=315USgAl2Ks"),
];

/// Open one of [`LINKS`] in the default browser.
#[tauri::command]
fn open_link(app: AppHandle, which: String) -> Result<(), String> {
    let url = LINKS.iter().find(|(id, _)| *id == which).map(|(_, u)| *u).ok_or("lien inconnu")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_profiles_dir(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let dir = state.dir()?;
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

// ---- plumbing ------------------------------------------------------------------

/// Show the main window, creating it if it was closed (to the tray, its web view is freed).
fn show_main(app: &AppHandle) {
    let w = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else { return };
            match tauri::WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build()) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("main window: {e}");
                    return;
                }
            }
        }
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

/// Mirror engine events into `Status` and forward them to the window.
fn forward_events(app: AppHandle, rx: std::sync::mpsc::Receiver<UiEvent>, status: Arc<Mutex<Status>>) {
    std::thread::Builder::new()
        .name("g13-ui-events".into())
        .spawn(move || {
            for ev in rx {
                // outside the status lock: toggle_overlay reads the status itself
                if ev == UiEvent::OverlayToggle {
                    toggle_overlay(&app);
                    continue;
                }
                let out = {
                    let mut s = status.lock().unwrap();
                    match ev {
                        UiEvent::Key { key, pressed } => Some(Ev::Key { key: key.id().into(), pressed }),
                        UiEvent::StickDir { dir, pressed } => Some(Ev::Key { key: dir.id().into(), pressed }),
                        UiEvent::BankChanged(b) => {
                            s.bank = format!("{b:?}");
                            Some(Ev::Bank { bank: s.bank.clone() })
                        }
                        UiEvent::ActiveProfile(id) => {
                            s.active_profile = id.clone();
                            Some(Ev::Profile { id })
                        }
                        UiEvent::Connected(c) => {
                            s.connected = c;
                            Some(Ev::Connected { value: c })
                        }
                        UiEvent::RecordMode(r) => {
                            s.recording = r;
                            Some(Ev::Record { value: r })
                        }
                        UiEvent::DisplayPower(d) => {
                            s.display_on = d;
                            Some(Ev::Display { value: d })
                        }
                        UiEvent::Pinned(id) => {
                            s.pinned = id.clone();
                            Some(Ev::Pinned { id })
                        }
                        UiEvent::LcdFrame(px) => {
                            s.lcd = Some(px.to_vec());
                            Some(Ev::Lcd { frame: px.to_vec() })
                        }
                        UiEvent::GamepadError(message) => Some(Ev::GamepadError { message }),
                        UiEvent::Mouse(st) => {
                            s.mouse = Some((*st).clone());
                            Some(Ev::Mouse { status: st })
                        }
                        UiEvent::Headset(st) => {
                            s.headset = Some((*st).clone());
                            Some(Ev::Headset { status: st })
                        }
                        UiEvent::OverlayToggle | UiEvent::LcdButton { .. } | UiEvent::ProfileChanged(_) => None,
                    }
                };
                if let Some(ev) = out {
                    let _ = app.emit("g13", ev);
                }
            }
        })
        .expect("spawn event forwarder");
}

/// Release builds have no console: log to `<config>/g13d/config/studio.log`
/// (overwritten at each start).
fn init_logging() {
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    b.format_timestamp_millis();
    if !cfg!(debug_assertions)
        && let Some(path) = g13_daemon::profiles::store::default_dir()
            .and_then(|d| d.parent().map(|p| p.join("studio.log")))
    {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(f) = std::fs::File::create(&path) {
            b.target(env_logger::Target::Pipe(Box::new(f)));
        }
    }
    b.init();
}

/// G Nexus's audio effect (equalizer), built by build.rs; written to disk when installed.
#[cfg(windows)]
const AUDIO_EFFECT_DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/g13_apo.dll"));

/// `G Nexus.exe --audio-setup install <result file> <output {guid}>` (or `uninstall <result file>`):
/// run as administrator by [`audio_eq_install`]; the error, if any, goes to the result file.
#[cfg(windows)]
fn audio_setup_mode(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("--audio-setup") {
        return None;
    }
    let result_file = args.get(3).map(PathBuf::from);
    let r = match (args.get(2).map(String::as_str), args.get(4)) {
        (Some("install"), Some(endpoint)) => {
            let dll = std::env::temp_dir().join("g13_apo_setup.dll");
            std::fs::write(&dll, AUDIO_EFFECT_DLL)
                .map_err(|e| e.to_string())
                .and_then(|()| g13_apo::setup::install(endpoint, &dll))
                .inspect(|()| {
                    let _ = std::fs::remove_file(&dll);
                })
        }
        (Some("uninstall"), _) => g13_apo::setup::uninstall(),
        _ => Err("commande d'installation inconnue".to_string()),
    };
    if let Some(f) = result_file {
        let _ = std::fs::write(f, r.as_ref().err().cloned().unwrap_or_default());
    }
    Some(if r.is_ok() { 0 } else { 1 })
}

/// Run the setup above as administrator (Windows asks the user) and wait for it.
#[cfg(windows)]
fn run_audio_setup(args: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let result = std::env::temp_dir().join(format!("g13-audio-setup-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&result);
    let code = g13_daemon::platform::windows::elevate::run_elevated(&exe, &format!("--audio-setup {args}").replace("{result}", &format!("\"{}\"", result.display())))
        .map_err(|e| e.detail())?;
    let msg = std::fs::read_to_string(&result).unwrap_or_default();
    let _ = std::fs::remove_file(&result);
    if code == 0 { Ok(()) } else if msg.is_empty() { Err(format!("installation interrompue (code {code})")) } else { Err(msg) }
}

/// Install G Nexus's equalizer on the headset's output (administrator rights, once).
#[tauri::command]
async fn audio_eq_install(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let name = state.status.lock().unwrap().headset.as_ref().and_then(|h| h.audio_device.clone());
        let name = name.unwrap_or_else(|| "PRO X Wireless".into());
        let endpoint = g13_daemon::platform::windows::audio::endpoint_key(&name)
            .map_err(|e| e.detail())?
            .ok_or("sortie audio du casque introuvable")?;
        tauri::async_runtime::spawn_blocking(move || run_audio_setup(&format!("install {{result}} {endpoint}")))
            .await
            .map_err(|e| e.to_string())??;
        if let Some(rt) = state.runtime.lock().unwrap().as_ref() {
            rt.headset(HeadsetCommand::Refresh);
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let name = state.status.lock().unwrap().headset.as_ref().and_then(|h| h.audio_device.clone());
        let name = name.unwrap_or_else(|| "PRO X Wireless".into());
        tauri::async_runtime::spawn_blocking(move || g13_daemon::headset::eq::install(&name))
            .await
            .map_err(|e| e.to_string())??;
        if let Some(rt) = state.runtime.lock().unwrap().as_ref() {
            rt.headset(HeadsetCommand::Refresh);
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = state;
        Err("égaliseur pas encore disponible sur ce système".into())
    }
}

/// Remove G Nexus's equalizer from the headset's output (administrator rights).
#[tauri::command]
async fn audio_eq_uninstall(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(|| run_audio_setup("uninstall {result}")).await.map_err(|e| e.to_string())??;
        if let Some(rt) = state.runtime.lock().unwrap().as_ref() {
            rt.headset(HeadsetCommand::Refresh);
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(g13_daemon::headset::eq::uninstall).await.map_err(|e| e.to_string())??;
        if let Some(rt) = state.runtime.lock().unwrap().as_ref() {
            rt.headset(HeadsetCommand::Refresh);
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = state;
        Err("égaliseur pas encore disponible sur ce système".into())
    }
}

fn main() {
    #[cfg(windows)]
    if let Some(code) = audio_setup_mode(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
    init_logging();

    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let autostarted = has("--autostart");
    // settings live next to the default profile directory (read before the
    // runtime starts: they carry the overlay key)
    let settings_path = Settings::path(g13_daemon::profiles::store::default_dir().as_deref());
    let settings = Settings::load(settings_path.as_deref());
    let opts = RuntimeOptions {
        dry_run: has("--dry-run"),
        poll: has("--poll"),
        overlay_key: parse_overlay_key(settings.overlay_key.as_deref()),
        ..RuntimeOptions::default()
    };

    let status = Arc::new(Mutex::new(Status {
        bank: "M1".into(),
        display_on: true,
        dry_run: opts.dry_run,
        ..Status::default()
    }));
    let (runtime, ui_rx) = match Runtime::start(opts) {
        Ok((rt, rx)) => (Some(rt), Some(rx)),
        Err(e) => {
            log::error!("{e}");
            status.lock().unwrap().error = Some(e);
            (None, None)
        }
    };
    let profiles_dir = runtime
        .as_ref()
        .and_then(|r| r.profiles_dir().map(PathBuf::from))
        .or_else(g13_daemon::profiles::store::default_dir);
    let start_hidden = autostarted && settings.start_minimized;

    let state = AppState {
        runtime: Mutex::new(runtime),
        profiles_dir,
        status: status.clone(),
        settings: Mutex::new(settings),
        settings_path,
        ui_lang: Mutex::new("fr".into()),
        recorder: Mutex::new(None),
        overlay_pending: Mutex::new(None),
    };

    let app = tauri::Builder::default()
        // Must be first: a second launch just brings this window forward
        // (two instances would fight over the G13).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_profiles,
            load_profile,
            save_profile,
            create_profile,
            delete_profile,
            get_settings,
            set_settings,
            open_profiles_dir,
            app_version,
            open_link,
            key_names,
            set_preview,
            conflicting_software,
            close_conflicting_software,
            device_access,
            install_device_access,
            restart_app,
            import_lgs,
            export_lgs,
            close_overlay,
            overlay_ready,
            backup_now,
            assign_game,
            set_pinned,
            detected_apps,
            pick_exe,
            gamepad_status,
            get_mouse,
            get_headset,
            set_headset_sidetone,
            set_headset_sleep,
            set_headset_eq,
            set_headset_volume,
            save_eq_profile,
            export_eq_profile,
            import_eq_profile,
            delete_eq_profile,
            use_eq_profile,
            set_eq_default,
            pin_eq_profile,
            audio_eq_install,
            audio_eq_uninstall,
            set_headset_mute,
            apply_mouse,
            preview_mouse,
            select_mouse_profile,
            export_mouse_profile,
            set_mouse_host,
            record_start,
            macro_library,
            save_library_macro,
            delete_library_macro,
            set_mouse_soft_default,
            pin_mouse_soft,
            record_stop,
            save_mouse_soft,
            delete_mouse_soft,
            import_mouse_profile,
            enable_mouse_profile,
            revert_mouse,
            set_language,
            ui_language,
            export_config,
            import_config,
            reset_config,
            open_backups_dir,
        ])
        .setup(move |app| {
            // The "start with Windows" entry stores the exe path: re-register it so it
            // follows the installed exe (e.g. after an update or a move). Not in debug
            // builds, so a dev build never hijacks the installed one's entry.
            #[cfg(not(debug_assertions))]
            {
                let launcher = app.autolaunch();
                if launcher.is_enabled().unwrap_or(false) {
                    let _ = launcher.disable();
                    if let Err(e) = enable_autostart(app.handle()) {
                        log::warn!("autostart refresh failed: {e}");
                    }
                }
            }

            let menu = tray_menu(app.handle(), "fr")?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("G Nexus")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // In-game overlay: borderless, transparent, always on top, outside
            // the taskbar; created hidden and shown by the overlay G-key. Works
            // over windowed / borderless-fullscreen games (an exclusive
            // fullscreen game is minimised by Windows while it is shown).
            spawn_auto_backup(app.handle().clone());

            // SIGTERM / SIGHUP / Ctrl+C (logout, kill): the same clean exit as « Quitter »
            // (RunEvent::Exit releases every key).
            #[cfg(unix)]
            {
                let handle = app.handle().clone();
                if let Err(e) = ctrlc::set_handler(move || handle.exit(0)) {
                    log::warn!("no signal handler: {e}");
                }
            }

            if let Some(rx) = ui_rx {
                forward_events(app.handle().clone(), rx, status);
            }
            if !start_hidden {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Alt+F4 on the overlay: closed like G25 / Échap would. The main window
            // simply closes: its web view goes away, the app stays in the tray (see
            // ExitRequested) and the tray icon creates it again.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "overlay"
            {
                api.prevent_close();
                hide_overlay(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri app");

    app.run(|app, event| {
        // last window closed: stay in the tray if so chosen ("Quitter" passes a code)
        if let RunEvent::ExitRequested { code: None, api, .. } = &event
            && app.state::<AppState>().settings.lock().unwrap().close_to_tray
        {
            api.prevent_exit();
        }
        if let RunEvent::Exit = event {
            // Release every held key and stop the HID thread before leaving.
            if let Some(mut rt) = app.state::<AppState>().runtime.lock().unwrap().take() {
                rt.stop();
            }
        }
    });
}

#[cfg(all(test, target_os = "linux"))]
mod autostart_tests {
    use super::quote_desktop_exec;

    #[test]
    fn quotes_a_path_with_spaces() {
        let raw = "[Desktop Entry]\nType=Application\nExec=/home/u/Apps/G Nexus_1.1.1_amd64.AppImage --autostart\nTerminal=false";
        let fixed = quote_desktop_exec(raw);
        assert!(fixed.contains("Exec=\"/home/u/Apps/G Nexus_1.1.1_amd64.AppImage\" --autostart\n"));
        // already quoted: unchanged
        assert_eq!(quote_desktop_exec(&fixed), fixed);
    }

    #[test]
    fn escapes_special_characters() {
        let fixed = quote_desktop_exec("Exec=/a/$b\"c --autostart\n");
        assert_eq!(fixed, "Exec=\"/a/\\$b\\\"c\" --autostart\n");
    }
}
