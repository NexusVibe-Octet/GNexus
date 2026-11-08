//! Import / export of Logitech Gaming Software (LGS 9.x) XML profiles.
//!
//! What an LGS profile file looks like (namespace `…/Cassandra/2010.7/Profile`):
//!
//! ```xml
//! <profiles><profile name="WARDOGS" …>
//!   <target path="C:\GAMES\WARDOGS.EXE"/>
//!   <macros>
//!     <macro guid="{…}" name="Z"><keystroke …><key value="W"/></keystroke></macro>
//!     <macro guid="{…}" name="Clic gauche"><mousefunction …><do task="leftclick"/></mousefunction></macro>
//!   </macros>
//!   <assignments devicecategory="Logitech.Gaming.LeftHandedController">   <!-- the G13 -->
//!     <assignment contextid="G4" macroguid="{…}" shiftstate="1" original="false" backup="false"/>
//!   </assignments>
//!   <backlight devicemodel="Logitech.Gaming.LeftHandedController.G13">
//!     <mode color="#ffffff" shiftstate="1"/>
//!   </backlight>
//! </profile></profiles>
//! ```
//!
//! * `shiftstate` 1/2/3 = M1/M2/M3. `contextid` G1–G25 are ours as is (G25 =
//!   stick click); G26/G27/G28/G29 are the stick up/right/down/left buttons.
//! * For one key and one M-state, LGS keeps the previous binding with
//!   `backup="true"`: the active one is `backup="false"`.
//! * Key names are **physical positions** with US labels (`W` is AZERTY Z),
//!   exactly like our [`KeyCode`]s — no layout translation is needed.
//! * Supported macro kinds: `keystroke` (including modifiers and repetition),
//!   balanced `multikey` sequences (keys, basic mouse buttons and delays),
//!   `mousefunction`, `hotkeys` copy/paste/cut and known media keys. A single
//!   no-delay keyboard-only `multikey` chord without repetition is held with
//!   the G-key as described in the LGS manual, if its event order is representable.
//!   Unknown steps, unsupported repetition and unbalanced sequences are
//!   reported as a whole, never partly converted. Text blocks, Lua scripts and
//!   layout-dependent hotkeys such as undo are also reported, not guessed.
//!   Timed and mouse-containing chords still need an LGS / G13 comparison.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use indexmap::IndexMap;

use super::ProfileError;
use super::format::{ActionDto, BankDto, FileDto, MacroDto, ModeDto};
use super::rules::MatchRule;
use crate::engine::bindings::{Input, MBank};
use crate::engine::macros::MIN_LOOP_PERIOD;
use crate::keycode::{KeyChord, KeyCode, MouseButton};

const G13_CATEGORY: &str = "Logitech.Gaming.LeftHandedController";
const G13_MODEL: &str = "Logitech.Gaming.LeftHandedController.G13";
const LCORE_PATH: &str = r"C:\PROGRAM FILES\LOGITECH GAMING SOFTWARE\LCORE.EXE";

/// LGS context id ↔ our binding id.
const CONTEXTS: &[(&str, &str)] = &[
    ("G26", "STICK_UP"),
    ("G27", "STICK_RIGHT"),
    ("G28", "STICK_DOWN"),
    ("G29", "STICK_LEFT"),
];

/// LGS key name ↔ our `KeyCode` name, for names that differ. Letters, digits
/// and F1–F24 are identical. The first entry for a key is used on export;
/// the others are accepted on import (names compared without `_`/case).
const KEY_NAMES: &[(&str, &str)] = &[
    ("ESCAPE", "Escape"),
    ("TILDE", "Grave"),
    ("MINUS", "Minus"),
    ("EQUAL", "Equal"),
    ("EQUALS", "Equal"),
    ("BACKSPACE", "Backspace"),
    ("TAB", "Tab"),
    ("LBRACKET", "LeftBracket"),
    ("OPEN_BRACKET", "LeftBracket"),
    ("RBRACKET", "RightBracket"),
    ("CLOSE_BRACKET", "RightBracket"),
    ("BACKSLASH", "Backslash"),
    ("CAPSLOCK", "CapsLock"),
    ("SEMICOLON", "Semicolon"),
    ("QUOTE", "Apostrophe"),
    ("APOSTROPHE", "Apostrophe"),
    ("ENTER", "Enter"),
    ("RETURN", "Enter"),
    ("LSHIFT", "LeftShift"),
    ("RSHIFT", "RightShift"),
    ("COMMA", "Comma"),
    ("PERIOD", "Period"),
    ("SLASH", "Slash"),
    ("NON_US_SLASH", "IntlBackslash"),
    ("LCTRL", "LeftCtrl"),
    ("RCTRL", "RightCtrl"),
    ("LALT", "LeftAlt"),
    ("RALT", "RightAlt"),
    ("LGUI", "LeftMeta"),
    ("LWIN", "LeftMeta"),
    ("RGUI", "RightMeta"),
    ("RWIN", "RightMeta"),
    ("APPKEY", "Menu"),
    ("APPLICATION", "Menu"),
    ("SPACEBAR", "Space"),
    ("SPACE", "Space"),
    ("PRINTSCREEN", "PrintScreen"),
    ("SCROLLLOCK", "ScrollLock"),
    ("PAUSE", "Pause"),
    ("PAUSE_BREAK", "Pause"),
    ("INSERT", "Insert"),
    ("HOME", "Home"),
    ("PAGEUP", "PageUp"),
    ("DELETE", "Delete"),
    ("END", "End"),
    ("PAGEDOWN", "PageDown"),
    ("UP", "Up"),
    ("ARROW_UP", "Up"),
    ("DOWN", "Down"),
    ("ARROW_DOWN", "Down"),
    ("LEFT", "Left"),
    ("ARROW_LEFT", "Left"),
    ("RIGHT", "Right"),
    ("ARROW_RIGHT", "Right"),
    ("NUMLOCK", "NumLock"),
    ("NUMSLASH", "NumpadDivide"),
    ("NUMASTERISK", "NumpadMultiply"),
    ("NUMMULTIPLY", "NumpadMultiply"),
    ("NUMMINUS", "NumpadSubtract"),
    ("NUMPLUS", "NumpadAdd"),
    ("NUMENTER", "NumpadEnter"),
    ("NUMPERIOD", "NumpadDecimal"),
    ("NUMDECIMAL", "NumpadDecimal"),
    ("NUM0", "Numpad0"),
    ("NUM1", "Numpad1"),
    ("NUM2", "Numpad2"),
    ("NUM3", "Numpad3"),
    ("NUM4", "Numpad4"),
    ("NUM5", "Numpad5"),
    ("NUM6", "Numpad6"),
    ("NUM7", "Numpad7"),
    ("NUM8", "Numpad8"),
    ("NUM9", "Numpad9"),
];

/// LGS `mousefunction` task ↔ our mouse button.
const MOUSE_TASKS: &[(&str, MouseButton)] = &[
    ("leftclick", MouseButton::Left),
    ("rightclick", MouseButton::Right),
    ("middleclick", MouseButton::Middle),
    ("back", MouseButton::Back),
    ("forward", MouseButton::Forward),
];

/// LGS `hotkeys` tasks that do not depend on the keyboard layout.
const HOTKEYS: &[(&str, &str)] = &[("copy", "LeftCtrl+C"), ("paste", "LeftCtrl+V"), ("cut", "LeftCtrl+X")];

/// Known LGS media tasks ↔ our key names. Match whole normalized names:
/// e.g. `mediaplayer` opens a player and must not become `playpause`.
const MEDIA_TASKS: &[(&str, &str)] = &[
    ("playpause", "MediaPlayPause"),
    ("nexttrack", "MediaNext"),
    ("next", "MediaNext"),
    ("previoustrack", "MediaPrevious"),
    ("previous", "MediaPrevious"),
    ("prev", "MediaPrevious"),
    ("stop", "MediaStop"),
    ("mute", "VolumeMute"),
    ("volumeup", "VolumeUp"),
    ("volup", "VolumeUp"),
    ("volumedown", "VolumeDown"),
    ("voldown", "VolumeDown"),
];

fn norm(s: &str) -> String {
    s.chars().filter(|c| *c != '_' && *c != ' ').collect::<String>().to_ascii_uppercase()
}

fn lgs_to_key(name: &str) -> Option<KeyCode> {
    let n = norm(name);
    KEY_NAMES
        .iter()
        .find(|(lgs, _)| norm(lgs) == n)
        .and_then(|(_, ours)| KeyCode::parse(ours))
        .or_else(|| {
            // letters, digits, F1–F24
            let k = KeyCode::parse(&n)?;
            (n.len() == 1 || (n.starts_with('F') && n[1..].parse::<u8>().is_ok())).then_some(k)
        })
}

fn key_to_lgs(key: KeyCode) -> Option<String> {
    if let Some((lgs, _)) = KEY_NAMES.iter().find(|(_, ours)| KeyCode::parse(ours) == Some(key)) {
        return Some((*lgs).to_string());
    }
    let n = key.name();
    let simple = n.len() == 1 || (n.starts_with('F') && n[1..].parse::<u8>().is_ok());
    simple.then(|| n.to_ascii_uppercase())
}

/// First child element named `name` (namespaces ignored).
fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.tag_name().name() == name)
}

/// Result of an import: the new profile plus what could not be converted.
#[derive(Debug, Clone)]
pub struct LgsImport {
    pub dto: FileDto,
    pub skipped: Vec<String>,
}

/// Convert an LGS profile (XML text) into our file model.
pub fn import(xml: &str) -> Result<LgsImport, ProfileError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ProfileError(format!("XML invalide : {e}")))?;
    let profile = doc
        .descendants()
        .find(|n| n.has_tag_name("profile") || n.tag_name().name() == "profile")
        .ok_or_else(|| ProfileError("aucun <profile> dans ce fichier".into()))?;
    let mut skipped = Vec::new();
    let mut dto = FileDto { name: profile.attribute("name").unwrap_or("Profil LGS").trim().to_string(), ..FileDto::default() };

    // executable → application rule (LGS points profiles without a game at LCore itself)
    if let Some(path) = child(profile, "target").and_then(|t| t.attribute("path")) {
        let exe = path.rsplit(['\\', '/']).next().unwrap_or("").trim();
        if !exe.is_empty() && !exe.eq_ignore_ascii_case("lcore.exe") {
            dto.rules.push(MatchRule::exe(exe));
        }
    }

    // Macros: GUID → (display name, converted action/sequence or reason).
    // A duplicate GUID is ambiguous: never pick one definition silently.
    let mut macros: HashMap<String, (String, Result<ConvertedMacro, String>)> = HashMap::new();
    if let Some(list) = child(profile, "macros") {
        for m in list.children().filter(|c| c.tag_name().name() == "macro") {
            let (Some(guid), name) = (m.attribute("guid"), m.attribute("name").unwrap_or("")) else { continue };
            match macros.entry(guid.to_uppercase()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((name.to_string(), convert_macro(m)));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    e.get_mut().1 = Err("identifiant de macro dupliqué dans le fichier".into());
                }
            }
        }
    }

    // G13 assignments: keep the active one (backup="false"), custom before default
    let mut chosen: IndexMap<(MBank, String), (bool, String)> = IndexMap::new();
    for group in profile
        .children()
        .filter(|c| c.tag_name().name() == "assignments" && c.attribute("devicecategory") == Some(G13_CATEGORY))
    {
        for a in group.children().filter(|c| c.tag_name().name() == "assignment") {
            if a.attribute("backup") == Some("true") {
                continue;
            }
            let (Some(ctx), Some(guid)) = (a.attribute("contextid"), a.attribute("macroguid")) else { continue };
            let bank = match a.attribute("shiftstate") {
                Some("1") => MBank::M1,
                Some("2") => MBank::M2,
                Some("3") => MBank::M3,
                _ => continue,
            };
            let custom = a.attribute("original") != Some("true");
            let id = CONTEXTS.iter().find(|(c, _)| *c == ctx).map_or(ctx.to_string(), |(_, ours)| (*ours).to_string());
            let key = (bank, id);
            if chosen.get(&key).is_none_or(|(prev_custom, _)| custom || !prev_custom) {
                chosen.insert(key, (custom, guid.to_uppercase()));
            }
        }
    }
    // Only the selected G13 assignments acquire macro definitions. Reusing the
    // same GUID reuses one definition; different GUIDs never overwrite each
    // other, even when LGS gives them the same name.
    let mut assigned_names: HashMap<String, String> = HashMap::new();
    for ((bank, id), (_, guid)) in chosen {
        if !Input::from_id(&id).is_some_and(|input| input.slot().is_some()) {
            skipped.push(format!("{bank:?} {id} : touche G13 non prise en charge"));
            continue;
        }
        match macros.get(&guid) {
            Some((name, Ok(converted))) => {
                let action = match converted {
                    ConvertedMacro::Direct(a) => a.clone(),
                    ConvertedMacro::Sequence(m) => {
                        let assigned = assigned_names.entry(guid).or_insert_with(|| {
                            let unique = unique_macro_name(name, &dto.macros);
                            dto.macros.insert(unique.clone(), m.clone());
                            unique
                        });
                        ActionDto::Macro { name: assigned.clone() }
                    }
                };
                dto.banks.get_mut(bank).keys.insert(id, action);
            }
            Some((name, Err(why))) => skipped.push(format!("{bank:?} {id} « {name} » : {why}")),
            None => skipped.push(format!("{bank:?} {id} : macro introuvable dans le fichier")),
        }
    }

    // A Lua script can affect keys independently of any individual binding.
    // Even LGS's default template is reported rather than assumed harmless.
    if profile.children().filter(|c| c.tag_name().name() == "script").any(|script| {
        script.attributes().len() > 0
            || script.descendants().any(|n| n.is_element() && n != script || n.is_text() && !n.text().unwrap_or("").trim().is_empty())
    }) {
        skipped.push("script Lua du profil non importé (y compris le modèle LGS par défaut)".into());
    }

    // backlight colour per M-state
    if let Some(bl) = profile
        .children()
        .find(|c| c.tag_name().name() == "backlight" && c.attribute("devicemodel") == Some(G13_MODEL))
    {
        for m in bl.children().filter(|c| c.tag_name().name() == "mode") {
            let bank = match m.attribute("shiftstate") {
                Some("1") => MBank::M1,
                Some("2") => MBank::M2,
                Some("3") => MBank::M3,
                _ => continue,
            };
            if let Some(c) = m.attribute("color").filter(|c| c.len() == 7 && c.starts_with('#')) {
                dto.banks.get_mut(bank).color = Some(c.to_uppercase());
            }
        }
    }

    // banks in G-key order, like files written by the app
    for bank in MBank::ALL {
        sort_keys(dto.banks.get_mut(bank));
    }
    Ok(LgsImport { dto, skipped })
}

fn sort_keys(b: &mut BankDto) {
    let order = |id: &str| {
        let ids = ["STICK_UP", "STICK_DOWN", "STICK_LEFT", "STICK_RIGHT"];
        id.strip_prefix('G')
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_else(|| 100 + ids.iter().position(|s| *s == id).unwrap_or(9) as u32)
    };
    b.keys.sort_by(|a, _, c, _| order(a).cmp(&order(c)));
}

/// The direct bindings are kept as before; sequences need an entry in the
/// profile's macro table. Deferring naming until assignment avoids collecting
/// unbound commands or macros for other Logitech devices.
#[derive(Debug)]
enum ConvertedMacro {
    Direct(ActionDto),
    Sequence(MacroDto),
}

/// LGS's repeat interval defaults to 25 ms when not present. The delay is
/// between iterations (not between the individual steps).
#[derive(Debug, Clone, Copy)]
enum Playback {
    Once,
    Repeat { mode: ModeDto, delay_ms: u32 },
}

fn playback(m: roxmltree::Node<'_, '_>) -> Result<Playback, String> {
    for a in m.attributes() {
        if a.name().to_ascii_lowercase().starts_with("repeat")
            && (a.namespace().is_some() || !matches!(a.name(), "repeatmode" | "repeatdelay"))
        {
            return Err(format!("option de répétition « {} » non prise en charge", a.name()));
        }
    }
    let delay_ms = m.attribute("repeatdelay").map(|d| {
        d.parse::<u32>().map_err(|_| format!("délai de répétition « {d} » invalide"))
    }).transpose()?;
    match m.attribute("repeatmode").map(str::to_ascii_lowercase).as_deref() {
        None | Some("none") => Ok(Playback::Once),
        Some("pressed") => Ok(Playback::Repeat { mode: ModeDto::RepeatWhileHeld, delay_ms: delay_ms.unwrap_or(25) }),
        Some("toggle") => Ok(Playback::Repeat { mode: ModeDto::Toggle, delay_ms: delay_ms.unwrap_or(25) }),
        Some(other) => Err(format!("mode de répétition « {other} » non pris en charge")),
    }
}

/// Prefix also prevents names such as `__proto__` from becoming special keys
/// in the GUI's JavaScript object; suffix collisions deterministically.
fn unique_macro_name(name: &str, existing: &IndexMap<String, MacroDto>) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let name = cleaned.trim();
    let base = format!("LGS: {}", if name.is_empty() { "Macro" } else { name });
    (1u64..).map(|n| if n == 1 { base.clone() } else { format!("{base} ({n})") })
        .find(|n| !existing.contains_key(n))
        .expect("unbounded macro names")
}

/// Validate the attributes and content of a known LGS step. Unknown details
/// might change its meaning, so reject the whole sequence rather than guessing.
fn leaf(n: roxmltree::Node<'_, '_>, allowed: &[&str]) -> Result<(), String> {
    if let Some(a) = n.attributes().find(|a| a.namespace().is_some() || !allowed.contains(&a.name())) {
        return Err(format!("attribut « {} » non pris en charge dans <{}>", a.name(), n.tag_name().name()));
    }
    if n.children().any(|c| c.is_element() || c.is_text() && !c.text().unwrap_or("").trim().is_empty()) {
        return Err(format!("contenu inattendu dans <{}>", n.tag_name().name()));
    }
    Ok(())
}

fn container(n: roxmltree::Node<'_, '_>) -> Result<(), String> {
    if n.attributes().len() != 0 || n.children().any(|c| c.is_text() && !c.text().unwrap_or("").trim().is_empty()) {
        return Err(format!("contenu ou attribut inattendu dans <{}>", n.tag_name().name()));
    }
    Ok(())
}

fn keystroke(body: roxmltree::Node<'_, '_>) -> Result<KeyChord, String> {
    container(body)?;
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for k in body.children().filter(|c| c.is_element()) {
        if k.tag_name().namespace() != body.tag_name().namespace() {
            return Err("espace de noms inattendu dans keystroke".into());
        }
        let tag = k.tag_name().name();
        if !matches!(tag, "key" | "modifier") {
            return Err(format!("élément <{tag}> non pris en charge dans keystroke"));
        }
        leaf(k, &["value"])?;
        let v = k.attribute("value").unwrap_or("");
        let key = lgs_to_key(v).ok_or_else(|| format!("touche LGS « {v} » inconnue"))?;
        if tag == "modifier" && !key.is_modifier() {
            return Err(format!("modificateur LGS « {v} » invalide"));
        }
        if !seen.insert(key) {
            return Err(format!("touche LGS « {v} » dupliquée"));
        }
        keys.push(key);
    }
    if keys.is_empty() {
        return Err("frappe sans touche".into());
    }
    Ok(KeyChord::new(keys))
}

/// Finish only after every step has been validated. The engine has a 10 ms
/// minimum loop period: reject a faster LGS loop if it would change its timing.
fn sequence(play: Playback, mut steps: Vec<String>, wait_ms: u64) -> Result<ConvertedMacro, String> {
    let mode = match play {
        Playback::Once => ModeDto::Once,
        Playback::Repeat { mode, delay_ms } => {
            let min_ms = MIN_LOOP_PERIOD.as_millis() as u64;
            if wait_ms.saturating_add(u64::from(delay_ms)) < min_ms {
                return Err(format!("répétition trop rapide pour le moteur (minimum {min_ms} ms par cycle)"));
            }
            steps.push(format!("wait {delay_ms}"));
            mode
        }
    };
    Ok(ConvertedMacro::Sequence(MacroDto { mode, steps }))
}

fn direction(n: roxmltree::Node<'_, '_>) -> Result<bool, String> {
    match n.attribute("direction").map(str::to_ascii_lowercase).as_deref() {
        Some("down") => Ok(true),
        Some("up") => Ok(false),
        _ => Err(format!("direction invalide dans <{}>", n.tag_name().name())),
    }
}

fn multikey(body: roxmltree::Node<'_, '_>, play: Playback) -> Result<ConvertedMacro, String> {
    container(body)?;
    let mut steps = Vec::new();
    let mut keys = HashSet::new();
    let mut buttons = HashSet::new();
    let mut wait_ms = 0u64;
    let mut has_event = false;
    // LGS holds a plain multikey chord until the physical G-key is released.
    // Recognise the documented, unambiguous no-delay keyboard-only case.
    let mut plain_chord = true;
    let mut saw_release = false;
    let mut chord_down = Vec::new();
    let mut chord_up = Vec::new();
    for n in body.children().filter(|c| c.is_element()) {
        if n.tag_name().namespace() != body.tag_name().namespace() {
            return Err("espace de noms inattendu dans multikey".into());
        }
        match n.tag_name().name() {
            "key" => {
                leaf(n, &["value", "direction"])?;
                let v = n.attribute("value").unwrap_or("");
                let k = lgs_to_key(v).ok_or_else(|| format!("touche LGS « {v} » inconnue"))?;
                let down = direction(n)?;
                if if down { !keys.insert(k) } else { !keys.remove(&k) } {
                    return Err(format!("appui/relâchement incohérent de la touche « {v} »"));
                }
                if down {
                    if saw_release {
                        plain_chord = false;
                    }
                    chord_down.push(k);
                } else {
                    saw_release = true;
                    chord_up.push(k);
                }
                steps.push(format!("{} {}", if down { "down" } else { "up" }, k.name()));
                has_event = true;
            }
            "mousebutton" => {
                plain_chord = false;
                leaf(n, &["button", "direction"])?;
                let v = n.attribute("button").unwrap_or("");
                let b = match v.to_ascii_lowercase().as_str() {
                    "left" => MouseButton::Left,
                    "right" => MouseButton::Right,
                    "middle" => MouseButton::Middle,
                    _ => return Err(format!("bouton de souris LGS « {v} » non pris en charge")),
                };
                let down = direction(n)?;
                if if down { !buttons.insert(b) } else { !buttons.remove(&b) } {
                    return Err(format!("appui/relâchement incohérent du bouton « {v} »"));
                }
                steps.push(format!("mouse_{} {}", if down { "down" } else { "up" }, b.name()));
                has_event = true;
            }
            "delay" => {
                // Preserve explicit timing as a sequence. Its interaction
                // with LGS hold behavior still needs a Windows hardware test.
                plain_chord = false;
                leaf(n, &["milliseconds"])?;
                let v = n.attribute("milliseconds").unwrap_or("");
                let ms = v.parse::<u32>().map_err(|_| format!("délai LGS « {v} » invalide"))?;
                wait_ms = wait_ms.saturating_add(u64::from(ms));
                steps.push(format!("wait {ms}"));
            }
            other => return Err(format!("étape multikey <{other}> non prise en charge")),
        }
    }
    if !has_event {
        return Err("séquence multikey vide".into());
    }
    if !keys.is_empty() || !buttons.is_empty() {
        return Err("séquence multikey avec touche ou bouton non relâché".into());
    }
    if matches!(play, Playback::Once) && plain_chord {
        // KeyChord sorts modifiers first and releases in reverse press order.
        // Refuse a chord we cannot reproduce exactly instead of playing it as
        // an immediate sequence or silently changing its event order.
        let chord = KeyChord::new(chord_down.clone());
        if chord.keys() != chord_down || !chord_down.iter().rev().eq(chord_up.iter()) {
            return Err("ordre d'appui/relâchement de l'accord multikey non pris en charge".into());
        }
        return Ok(ConvertedMacro::Direct(ActionDto::Chord(chord.to_string())));
    }
    sequence(play, steps, wait_ms)
}

fn convert_macro(m: roxmltree::Node<'_, '_>) -> Result<ConvertedMacro, String> {
    let play = playback(m)?;
    if m.children().any(|c| c.is_text() && !c.text().unwrap_or("").trim().is_empty()) {
        return Err("contenu inattendu dans la macro".into());
    }
    let mut bodies = m.children().filter(|c| c.is_element());
    let Some(body) = bodies.next() else { return Err("macro vide".into()) };
    if bodies.next().is_some() {
        return Err("plusieurs actions dans une même macro".into());
    }
    match body.tag_name().name() {
        "keystroke" => {
            let chord = keystroke(body)?;
            if matches!(play, Playback::Once) {
                return Ok(ConvertedMacro::Direct(ActionDto::Chord(chord.to_string())));
            }
            let mut steps: Vec<String> = chord.keys().iter().map(|k| format!("down {}", k.name())).collect();
            steps.extend(chord.keys().iter().rev().map(|k| format!("up {}", k.name())));
            sequence(play, steps, 0)
        }
        "multikey" => multikey(body, play),
        _ if !matches!(play, Playback::Once) => Err("répétition de ce type de commande non prise en charge".into()),
        _ => convert_direct(body).map(ConvertedMacro::Direct),
    }
}

fn direct_task<'a>(body: roxmltree::Node<'a, '_>) -> Result<&'a str, String> {
    container(body)?;
    let mut children = body.children().filter(|c| c.is_element());
    let Some(do_) = children.next() else { return Err("commande sans action".into()) };
    if children.next().is_some() || do_.tag_name().name() != "do" || do_.tag_name().namespace() != body.tag_name().namespace() {
        return Err("action ou espace de noms inattendu dans la commande".into());
    }
    leaf(do_, &["task"])?;
    do_.attribute("task").filter(|t| !t.is_empty()).ok_or_else(|| "commande sans action".into())
}

fn convert_direct(body: roxmltree::Node<'_, '_>) -> Result<ActionDto, String> {
    match body.tag_name().name() {
        "mousefunction" => {
            let t = direct_task(body)?;
            MOUSE_TASKS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(t))
                .map(|(_, b)| ActionDto::Mouse { mouse: b.name().into() })
                .ok_or_else(|| format!("fonction souris « {t} » non prise en charge"))
        }
        "hotkeys" => {
            let t = direct_task(body)?;
            HOTKEYS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(t))
                .map(|(_, chord)| ActionDto::Chord((*chord).into()))
                .ok_or_else(|| format!("raccourci « {t} » non importé (dépend de la disposition du clavier)"))
        }
        "function" | "media" => {
            let task = direct_task(body)?;
            let t = task.to_ascii_lowercase().replace(['_', ' '], "");
            MEDIA_TASKS
                .iter()
                .find(|(n, _)| t == *n)
                .map(|(_, k)| ActionDto::Chord((*k).into()))
                .ok_or_else(|| format!("fonction « {task} » non prise en charge"))
        }
        "natural" => Err("fonction d'origine de la touche (sans équivalent)".into()),
        other => Err(format!("macro de type « {other} » non prise en charge (texte ou script)")),
    }
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Random-looking GUID `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` (no OS call:
/// std's per-process random hasher keys, mixed with a counter).
fn guid(counter: &mut u64) -> String {
    use std::hash::{BuildHasher, Hasher};
    let st = std::collections::hash_map::RandomState::new();
    let n = *counter;
    *counter += 1;
    let part = |salt: u64| {
        let mut h = st.build_hasher();
        h.write_u64(n);
        h.write_u64(salt);
        h.finish()
    };
    let (a, b) = (part(1), part(2));
    format!(
        "{{{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}}}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0x0FFF,
        0x8000 | (b >> 48 & 0x3FFF),
        b & 0xFFFF_FFFF_FFFF
    )
}

/// Body of an LGS `<macro>` for one of our actions, or why it can't be exported.
fn export_action(a: &ActionDto) -> Result<(String, String), String> {
    match a {
        ActionDto::Chord(s) => {
            let chord = KeyChord::parse(s).ok_or_else(|| format!("combinaison invalide « {s} »"))?;
            let names = chord
                .keys()
                .iter()
                .map(|k| key_to_lgs(*k).ok_or_else(|| format!("la touche « {k} » n'a pas d'équivalent LGS connu")))
                .collect::<Result<Vec<_>, _>>()?;
            let mut body = String::from(
                "        <keystroke xmlns=\"http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke\">\n",
            );
            for n in &names {
                let _ = writeln!(body, "          <key value=\"{n}\"/>");
            }
            body.push_str("        </keystroke>\n");
            Ok((chord.to_string(), body))
        }
        ActionDto::Mouse { mouse } => {
            let b = MouseButton::parse(mouse).ok_or_else(|| format!("bouton « {mouse} » inconnu"))?;
            let (task, _) = MOUSE_TASKS.iter().find(|(_, mb)| *mb == b).expect("all buttons are listed");
            Ok((
                mouse.clone(),
                format!(
                    "        <mousefunction xmlns=\"http://www.logitech.com/Cassandra/2010.1/Macros/MouseFunction\">\n          <do task=\"{task}\"/>\n        </mousefunction>\n"
                ),
            ))
        }
        ActionDto::Wheel { .. } => Err("la molette n'a pas d'équivalent LGS connu".into()),
        ActionDto::Macro { name } => Err(format!("la macro « {name} » (séquence) n'est pas exportable vers LGS")),
        ActionDto::Pad { pad } => Err(format!("le bouton de manette « {pad} » n'est pas exportable vers LGS")),
        // LGS has one action per key: the tap one
        ActionDto::TapDance { tap: Some(tap), .. } => export_action(tap),
        ActionDto::TapDance { .. } => Err("touche multi-actions sans appui court : pas d'équivalent LGS".into()),
    }
}

/// Convert our profile to an LGS XML document. Returns the XML and the
/// bindings that could not be exported.
pub fn export(dto: &FileDto) -> (String, Vec<String>) {
    let mut counter = 0u64;
    let mut skipped = Vec::new();
    let mut macro_xml = String::new();
    let mut by_content: HashMap<String, String> = HashMap::new(); // body → guid
    let mut assigns = String::new();

    for bank in MBank::ALL {
        let shift = bank.index() + 1;
        for (id, action) in &dto.banks.get(bank).keys {
            let ctx = CONTEXTS.iter().find(|(_, ours)| ours == id).map_or(id.as_str(), |(c, _)| c);
            match export_action(action) {
                Ok((name, body)) => {
                    let g = match by_content.get(&body) {
                        Some(g) => g.clone(),
                        None => {
                            let g = guid(&mut counter);
                            let _ = write!(
                                macro_xml,
                                "      <macro guid=\"{g}\" color=\"4278246655\" name=\"{}\" hidden=\"false\">\n{body}      </macro>\n",
                                xml_escape(&name)
                            );
                            by_content.insert(body, g.clone());
                            g
                        }
                    };
                    let _ = writeln!(
                        assigns,
                        "      <assignment contextid=\"{ctx}\" macroguid=\"{g}\" shiftstate=\"{shift}\" original=\"false\" backup=\"false\"/>"
                    );
                }
                Err(why) => skipped.push(format!("{bank:?} {id} : {why}")),
            }
        }
    }

    let target = dto.rules.iter().find_map(|r| r.exe.clone()).unwrap_or_else(|| LCORE_PATH.to_string());
    let name = xml_escape(&dto.name);
    let mut out = String::new();
    let _ = write!(
        out,
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<profiles xmlns=\"http://www.logitech.com/Cassandra/2010.7/Profile\">\n  <profile guid=\"{}\" lock=\"0\" name=\"{name}\" gkeysdk=\"0\" launchable=\"1\" gameid=\"\" gpasupported=\"0\">\n    <description>{name}</description>\n    <target path=\"{}\"/>\n    <signature value=\"\" key=\"\" name=\"\" executable=\"\"/>\n    <macros>\n{macro_xml}    </macros>\n    <assignments devicecategory=\"{G13_CATEGORY}\">\n{assigns}    </assignments>\n",
        guid(&mut counter),
        xml_escape(&target)
    );
    let colors: Vec<(usize, &String)> =
        MBank::ALL.iter().filter_map(|b| dto.banks.get(*b).color.as_ref().map(|c| (b.index() + 1, c))).collect();
    if !colors.is_empty() {
        let _ = writeln!(out, "    <backlight devicemodel=\"{G13_MODEL}\">");
        for (shift, c) in colors {
            let _ = writeln!(out, "      <mode color=\"{}\" shiftstate=\"{shift}\"/>", xml_escape(&c.to_lowercase()));
        }
        out.push_str("    </backlight>\n");
    }
    out.push_str("  </profile>\n</profiles>\n");
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::from_dto;

    const SAMPLE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<profiles xmlns="http://www.logitech.com/Cassandra/2010.7/Profile">
  <profile guid="{CFEDC0AE-7AAC-49F7-A9A3-2FEF319BE1FA}" lock="0" name="WARDOGS" gkeysdk="0" launchable="1">
    <description>WARDOGS</description>
    <target path="C:\GAMES\Wardogs\wardogs.exe"/>
    <macros>
      <macro guid="{00000000-0000-0000-0000-000000000001}" name="1" original="true"><keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke"><key value="1"/></keystroke></macro>
      <macro guid="{00000000-0000-0000-0000-000000000002}" name="ECHAP"><keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke"><key value="ESCAPE"/></keystroke></macro>
      <macro guid="{00000000-0000-0000-0000-000000000003}" name="Z"><keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke"><key value="W"/></keystroke></macro>
      <macro guid="{00000000-0000-0000-0000-000000000004}" name="Clic gauche"><mousefunction xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/MouseFunction"><do task="leftclick"/></mousefunction></macro>
      <macro guid="{00000000-0000-0000-0000-000000000005}" name="Copier"><hotkeys xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Hotkeys"><do task="copy"/></hotkeys></macro>
      <macro guid="{00000000-0000-0000-0000-000000000006}" name="Annuler"><hotkeys xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Hotkeys"><do task="undo"/></hotkeys></macro>
      <macro guid="{00000000-0000-0000-0000-000000000007}" name="Ctrl+Maj+Échap"><keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke"><key value="LCTRL"/><key value="LSHIFT"/><key value="ESCAPE"/></keystroke></macro>
      <macro guid="{00000000-0000-0000-0000-000000000008}" name="2 (PAVE NUM.)"><keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke"><key value="NUM2"/></keystroke></macro>
      <macro guid="{00000000-0000-0000-0000-000000000009}" name="Seq"><multikey xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Multikey"/></macro>
    </macros>
    <assignments devicecategory="Logitech.Gaming.Keyboard">
      <assignment contextid="G1" macroguid="{00000000-0000-0000-0000-000000000003}" shiftstate="1" original="false" backup="false"/>
    </assignments>
    <assignments devicecategory="Logitech.Gaming.LeftHandedController">
      <assignment contextid="G1" macroguid="{00000000-0000-0000-0000-000000000001}" shiftstate="1" original="false" backup="true"/>
      <assignment contextid="G1" macroguid="{00000000-0000-0000-0000-000000000001}" shiftstate="2" original="true" backup="false"/>
      <assignment contextid="G1" macroguid="{00000000-0000-0000-0000-000000000002}" shiftstate="1" original="false" backup="false"/>
      <assignment contextid="G4" macroguid="{00000000-0000-0000-0000-000000000003}" shiftstate="1" original="false" backup="false"/>
      <assignment contextid="G25" macroguid="{00000000-0000-0000-0000-000000000004}" shiftstate="1" original="false" backup="false"/>
      <assignment contextid="G26" macroguid="{00000000-0000-0000-0000-000000000003}" shiftstate="1" original="true" backup="false"/>
      <assignment contextid="G29" macroguid="{00000000-0000-0000-0000-000000000008}" shiftstate="3" original="false" backup="false"/>
      <assignment contextid="G2" macroguid="{00000000-0000-0000-0000-000000000005}" shiftstate="2" original="false" backup="false"/>
      <assignment contextid="G3" macroguid="{00000000-0000-0000-0000-000000000006}" shiftstate="2" original="false" backup="false"/>
      <assignment contextid="G5" macroguid="{00000000-0000-0000-0000-000000000007}" shiftstate="1" original="false" backup="false"/>
      <assignment contextid="G6" macroguid="{00000000-0000-0000-0000-000000000009}" shiftstate="1" original="false" backup="false"/>
    </assignments>
    <backlight devicemodel="Logitech.Gaming.LeftHandedController.G13">
      <mode color="#ff8000" shiftstate="1"/>
      <mode color="#ffffff" shiftstate="2"/>
    </backlight>
  </profile>
</profiles>"##;

    fn key(dto: &FileDto, bank: MBank, id: &str) -> Option<String> {
        dto.banks.get(bank).keys.get(id).map(|a| serde_json::to_string(a).unwrap())
    }

    // Small synthetic profiles use the same element forms as public LGS G13
    // exports (MarkJerde/Logitech-G13-Profiles and cheshire137/logitech-g13-profiles).
    fn profile_xml(macros: &str, assignments: &str, extra: &str) -> String {
        format!(r#"<profiles xmlns="http://www.logitech.com/Cassandra/2010.7/Profile">
          <profile name="Test"><macros>{macros}</macros>
          <assignments devicecategory="Logitech.Gaming.LeftHandedController">{assignments}</assignments>
          {extra}</profile></profiles>"#)
    }

    fn assignment(guid: &str, id: &str, shift: u8) -> String {
        format!(r#"<assignment macroguid="{guid}" contextid="{id}" shiftstate="{shift}" backup="false"/>"#)
    }

    fn sequence_xml(guid: &str, name: &str, attrs: &str, steps: &str) -> String {
        format!(r#"<macro guid="{guid}" name="{name}" {attrs}>
          <multikey xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/MultiKey">{steps}</multikey></macro>"#)
    }

    fn sequence_at<'a>(dto: &'a FileDto, bank: MBank, id: &str) -> (&'a str, &'a MacroDto) {
        match dto.banks.get(bank).keys.get(id) {
            Some(ActionDto::Macro { name }) => (name, dto.macros.get(name).expect("referenced macro exists")),
            other => panic!("expected a macro on {bank:?} {id}, got {other:?}"),
        }
    }

    #[test]
    fn lgs_simple_multikey_chord_is_held_until_the_g_key_is_released() {
        use crate::device::DeviceEvent;
        use crate::engine::{Engine, action::Action};
        use crate::platform::mock::{Injected, RecordingInjector};
        use g13_core::G13Key;
        use std::sync::{Arc, mpsc::channel};

        // The exact example documented by Logitech's G13 LGS manual: a single
        // chord without delays must mirror the physical G-key's press/release.
        let keys = r#"<key value="A" direction="down"/><key value="X" direction="down"/>
            <key value="SPACEBAR" direction="down"/><key value="SPACEBAR" direction="up"/>
            <key value="X" direction="up"/><key value="A" direction="up"/>"#;
        let xml = profile_xml(&sequence_xml("a", "Held chord", "", keys), &assignment("a", "G1", 1), "");
        let imported = import(&xml).unwrap();
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
        let doc = from_dto(imported.dto).unwrap();
        assert!(matches!(doc.profile.bank(MBank::M1).get(G13Key::G1), Action::Key(_)));

        let rec = Arc::new(RecordingInjector::new());
        let (tx, _rx) = channel();
        let mut engine = Engine::new(rec.clone(), tx, doc.profile);
        engine.handle(DeviceEvent::Key { key: G13Key::G1, pressed: true });
        assert_eq!(rec.take(), vec![
            Injected::Key(KeyCode::A, true), Injected::Key(KeyCode::X, true), Injected::Key(KeyCode::Space, true),
        ]);
        // No up event may be sent until the physical key is released.
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(rec.take().is_empty());
        engine.handle(DeviceEvent::Key { key: G13Key::G1, pressed: false });
        assert_eq!(rec.take(), vec![
            Injected::Key(KeyCode::Space, false), Injected::Key(KeyCode::X, false), Injected::Key(KeyCode::A, false),
        ]);
        engine.handle(DeviceEvent::Key { key: G13Key::G1, pressed: true });
        let _ = rec.take(); // consume the new down events
        engine.handle(DeviceEvent::Disconnected);
        assert_eq!(rec.take(), vec![
            Injected::Key(KeyCode::Space, false), Injected::Key(KeyCode::X, false), Injected::Key(KeyCode::A, false),
        ], "disconnect must release the held chord");
    }

    #[test]
    fn lgs_single_key_and_modifier_chords_are_imported_as_held_keys() {
        let one = r#"<key value="W" direction="down"/><key value="W" direction="up"/>"#;
        let modified = r#"<key value="LCTRL" direction="down"/><key value="C" direction="down"/>
            <key value="C" direction="up"/><key value="LCTRL" direction="up"/>"#;
        let xml = profile_xml(
            &[sequence_xml("a", "Single", "", one), sequence_xml("b", "Modified", "", modified)].concat(),
            &[assignment("a", "G1", 1), assignment("b", "G2", 1)].concat(), "",
        );
        let imported = import(&xml).unwrap();
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
        assert_eq!(key(&imported.dto, MBank::M1, "G1").as_deref(), Some(r#""W""#));
        assert_eq!(key(&imported.dto, MBank::M1, "G2").as_deref(), Some(r#""LeftCtrl+C""#));
        assert!(imported.dto.macros.is_empty());
        let (exported, skipped) = export(&imported.dto);
        assert!(skipped.is_empty(), "{skipped:?}");
        let roundtrip = import(&exported).unwrap();
        assert_eq!(key(&roundtrip.dto, MBank::M1, "G1"), key(&imported.dto, MBank::M1, "G1"));
        assert_eq!(key(&roundtrip.dto, MBank::M1, "G2"), key(&imported.dto, MBank::M1, "G2"));
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn lgs_held_chords_with_unrepresentable_order_are_reported_not_reordered() {
        let wrong_release = r#"<key value="A" direction="down"/><key value="X" direction="down"/>
            <key value="A" direction="up"/><key value="X" direction="up"/>"#;
        let late_modifier = r#"<key value="A" direction="down"/><key value="LCTRL" direction="down"/>
            <key value="LCTRL" direction="up"/><key value="A" direction="up"/>"#;
        let xml = profile_xml(
            &[sequence_xml("a", "Release order", "", wrong_release), sequence_xml("b", "Modifier order", "", late_modifier)].concat(),
            &[assignment("a", "G1", 1), assignment("b", "G2", 1)].concat(), "",
        );
        let imported = import(&xml).unwrap();
        for id in ["G1", "G2"] {
            assert!(key(&imported.dto, MBank::M1, id).is_none());
            assert!(imported.skipped.iter().any(|s| s.contains(id)), "{id}: {:?}", imported.skipped);
        }
        assert_eq!(imported.skipped.len(), 2);
        assert!(imported.dto.macros.is_empty());
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn lgs_timed_and_multiple_keystrokes_keep_their_sequence_steps() {
        let timed = r#"<key value="A" direction="down"/><delay milliseconds="20"/>
            <key value="A" direction="up"/>"#;
        let taps = r#"<key value="A" direction="down"/><key value="A" direction="up"/>
            <key value="X" direction="down"/><key value="X" direction="up"/>"#;
        let xml = profile_xml(
            &[sequence_xml("a", "Timed", "", timed), sequence_xml("b", "Two taps", "", taps)].concat(),
            &[assignment("a", "G1", 1), assignment("b", "G2", 1)].concat(), "",
        );
        let imported = import(&xml).unwrap();
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
        let a = sequence_at(&imported.dto, MBank::M1, "G1").1;
        let b = sequence_at(&imported.dto, MBank::M1, "G2").1;
        assert_eq!(a.mode, ModeDto::Once);
        assert_eq!(a.steps, vec!["down A", "wait 20", "up A"]);
        assert_eq!(b.mode, ModeDto::Once);
        assert_eq!(b.steps, vec!["down A", "up A", "down X", "up X"]);
        from_dto(imported.dto).unwrap();
        // The LGS semantics of timed / mixed mouse chords remain unverified.
    }

    #[test]
    fn imports_multikey_steps_in_order_and_survives_profile_serialization() {
        let steps = r#"
          <key value="LCTRL" direction="down"/><delay milliseconds="81"/>
          <key value="C" direction="down"/><key value="C" direction="up"/>
          <key value="LCTRL" direction="up"/>
          <mousebutton button="left" direction="down"/><delay milliseconds="13"/>
          <mousebutton button="left" direction="up"/>
          <mousebutton button="right" direction="down"/><mousebutton button="right" direction="up"/>
          <mousebutton button="middle" direction="down"/><mousebutton button="middle" direction="up"/>
          <key value="PERIOD" direction="down"/><key value="PERIOD" direction="up"/>"#;
        let xml = profile_xml(&sequence_xml("a", "Ctrl and click", "", steps), &assignment("a", "G1", 1), "");
        let imported = import(&xml).unwrap();
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
        let (name, mac) = sequence_at(&imported.dto, MBank::M1, "G1");
        assert_eq!(name, "LGS: Ctrl and click");
        assert_eq!(mac.mode, ModeDto::Once);
        assert_eq!(mac.steps, vec![
            "down LeftCtrl", "wait 81", "down C", "up C", "up LeftCtrl", "mouse_down Left", "wait 13",
            "mouse_up Left", "mouse_down Right", "mouse_up Right", "mouse_down Middle", "mouse_up Middle",
            "down Period", "up Period",
        ]);
        let (_, export_skipped) = export(&imported.dto);
        assert_eq!(export_skipped.len(), 1, "sequences are not exported back to LGS");
        assert!(export_skipped[0].contains("Ctrl and click"));
        let original = from_dto(imported.dto.clone()).unwrap();
        let toml = crate::profiles::dto_to_toml(&imported.dto).unwrap();
        let restored = crate::profiles::from_toml(&toml).unwrap();
        assert_eq!(original, restored);
        let json = serde_json::to_string(&imported.dto).unwrap();
        let dto: FileDto = serde_json::from_str(&json).unwrap();
        assert_eq!(sequence_at(&dto, MBank::M1, "G1").1, mac);
    }

    #[test]
    fn imported_macro_plays_exact_events_using_the_mock_injector() {
        use crate::engine::action::Action;
        use crate::platform::mock::{Injected, RecordingInjector};
        use g13_core::G13Key;
        use std::sync::Arc;
        use std::time::Duration;

        let steps = r#"<key value="LCTRL" direction="down"/><key value="C" direction="down"/>
            <delay milliseconds="2"/><key value="C" direction="up"/><key value="LCTRL" direction="up"/>
            <mousebutton button="left" direction="down"/><mousebutton button="left" direction="up"/>"#;
        let xml = profile_xml(&sequence_xml("a", "test", "", steps), &assignment("a", "G1", 1), "");
        let doc = from_dto(import(&xml).unwrap().dto).unwrap();
        let Action::Macro(mac) = doc.profile.bank(MBank::M1).get(G13Key::G1) else { panic!("not a macro") };
        let rec = Arc::new(RecordingInjector::new());
        let handle = crate::engine::macros::spawn(rec.clone(), mac.clone(), false).unwrap();
        assert!(handle.wait(Duration::from_secs(2)));
        assert_eq!(rec.take(), vec![
            Injected::Key(KeyCode::LeftCtrl, true), Injected::Key(KeyCode::C, true),
            Injected::Key(KeyCode::C, false), Injected::Key(KeyCode::LeftCtrl, false),
            Injected::Button(MouseButton::Left, true), Injected::Button(MouseButton::Left, false),
        ]);
    }

    #[test]
    fn imported_repeat_cancellation_releases_a_held_modifier() {
        use crate::engine::action::Action;
        use crate::platform::mock::{Injected, RecordingInjector};
        use g13_core::G13Key;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let steps = r#"<key value="LSHIFT" direction="down"/><delay milliseconds="5000"/>
            <key value="LSHIFT" direction="up"/>"#;
        let xml = profile_xml(&sequence_xml("a", "test", r#"repeatmode="pressed" repeatdelay="25""#, steps), &assignment("a", "G1", 1), "");
        let doc = from_dto(import(&xml).unwrap().dto).unwrap();
        let Action::Macro(mac) = doc.profile.bank(MBank::M1).get(G13Key::G1) else { panic!("not a macro") };
        let rec = Arc::new(RecordingInjector::new());
        let handle = crate::engine::macros::spawn(rec.clone(), mac.clone(), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut events = Vec::new();
        while events.is_empty() && Instant::now() < deadline {
            events.extend(rec.take());
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.cancel();
        assert!(handle.wait(Duration::from_secs(2)));
        events.extend(rec.take());
        assert_eq!(events, vec![Injected::Key(KeyCode::LeftShift, true), Injected::Key(KeyCode::LeftShift, false)]);
    }

    #[test]
    fn shares_guids_but_never_overwrites_duplicate_names() {
        // Two successive taps are sequences rather than the held single chord.
        let steps_a = r#"<key value="A" direction="down"/><key value="A" direction="up"/><key value="A" direction="down"/><key value="A" direction="up"/>"#;
        let steps_b = r#"<key value="B" direction="down"/><key value="B" direction="up"/><key value="B" direction="down"/><key value="B" direction="up"/>"#;
        let macros = [
            sequence_xml("a", "Dup", "", steps_a), sequence_xml("b", "Dup", "", steps_b),
            sequence_xml("c", "Dup (2)", "", steps_a), sequence_xml("d", "__proto__", "", steps_a),
            sequence_xml("e", "", "", steps_b), sequence_xml("unused", "Unused", "", steps_a),
        ].concat();
        let assignments = [assignment("a", "G1", 1), assignment("a", "G2", 2), assignment("b", "G3", 1),
            assignment("c", "G4", 1), assignment("d", "G5", 1), assignment("e", "G6", 1)].concat();
        let imported = import(&profile_xml(&macros, &assignments, "")).unwrap();
        assert!(imported.skipped.is_empty());
        let a = sequence_at(&imported.dto, MBank::M1, "G1");
        assert_eq!(a.0, sequence_at(&imported.dto, MBank::M2, "G2").0);
        let b = sequence_at(&imported.dto, MBank::M1, "G3");
        assert_ne!(a.0, b.0);
        assert_ne!(a.1.steps, b.1.steps);
        assert_eq!(a.0, "LGS: Dup");
        assert_eq!(b.0, "LGS: Dup (2)");
        assert_eq!(sequence_at(&imported.dto, MBank::M1, "G4").0, "LGS: Dup (2) (2)");
        assert_eq!(sequence_at(&imported.dto, MBank::M1, "G5").0, "LGS: __proto__");
        assert_eq!(sequence_at(&imported.dto, MBank::M1, "G6").0, "LGS: Macro");
        assert_eq!(imported.dto.macros.len(), 5, "unassigned commands are not imported");
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn imports_repeat_modes_and_repeat_intervals_without_changing_timing() {
        let keys = r#"<key value="A" direction="down"/><key value="A" direction="up"/>"#;
        let slower_keys = r#"<key value="A" direction="down"/><delay milliseconds="10"/><key value="A" direction="up"/>"#;
        let click = r#"<mousebutton button="left" direction="down"/><mousebutton button="left" direction="up"/>"#;
        let macros = [sequence_xml("a", "Held", r#"repeatmode="pressed" repeatdelay="80""#, keys),
            sequence_xml("b", "Toggle", r#"repeatmode="toggle""#, keys),
            sequence_xml("c", "Slow body", r#"repeatmode="pressed" repeatdelay="0""#, slower_keys),
            sequence_xml("d", "No repeat", r#"repeatmode="none" repeatdelay="100""#, keys),
            sequence_xml("e", "Click", r#"repeatmode="pressed" repeatdelay="30""#, click)].concat();
        let assigns = [assignment("a", "G1", 1), assignment("b", "G2", 1), assignment("c", "G3", 1),
            assignment("d", "G4", 1), assignment("e", "G5", 1)].concat();
        let imported = import(&profile_xml(&macros, &assigns, "")).unwrap();
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
        let held = sequence_at(&imported.dto, MBank::M1, "G1").1;
        assert_eq!(held.mode, ModeDto::RepeatWhileHeld);
        assert_eq!(held.steps, vec!["down A", "up A", "wait 80"]);
        let toggle = sequence_at(&imported.dto, MBank::M1, "G2").1;
        assert_eq!(toggle.mode, ModeDto::Toggle);
        assert_eq!(toggle.steps, vec!["down A", "up A", "wait 25"], "LGS default");
        let slow = sequence_at(&imported.dto, MBank::M1, "G3").1;
        assert_eq!(slow.mode, ModeDto::RepeatWhileHeld);
        assert_eq!(slow.steps, vec!["down A", "wait 10", "up A", "wait 0"]);
        // LGS's one-key multikey without repetition is held, not tapped.
        assert_eq!(key(&imported.dto, MBank::M1, "G4").as_deref(), Some(r#""A""#));
        let click = sequence_at(&imported.dto, MBank::M1, "G5").1;
        assert_eq!(click.mode, ModeDto::RepeatWhileHeld);
        assert_eq!(click.steps, vec!["mouse_down Left", "mouse_up Left", "wait 30"]);
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn keystroke_modifiers_are_preserved_and_repeated_chords_become_macros() {
        let body = r#"<keystroke xmlns="http://www.logitech.com/Cassandra/2010.1/Macros/Keystroke">
          <key value="C"/><modifier value="LCTRL"/><modifier value="LSHIFT"/></keystroke>"#;
        let macros = format!(r#"<macro guid="a" name="Copy">{body}</macro>
            <macro guid="b" name="Again" repeatmode="pressed" repeatdelay="40">{body}</macro>"#);
        let assigns = [assignment("a", "G1", 1), assignment("b", "G2", 1)].concat();
        let imported = import(&profile_xml(&macros, &assigns, "")).unwrap();
        assert!(imported.skipped.is_empty());
        assert_eq!(key(&imported.dto, MBank::M1, "G1").as_deref(), Some(r#""LeftCtrl+LeftShift+C""#));
        let repeat = sequence_at(&imported.dto, MBank::M1, "G2").1;
        assert_eq!(repeat.mode, ModeDto::RepeatWhileHeld);
        assert_eq!(repeat.steps, vec!["down LeftCtrl", "down LeftShift", "down C", "up C", "up LeftShift", "up LeftCtrl", "wait 40"]);
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn unsafe_or_unrecognised_steps_skip_the_whole_macro_and_report_it() {
        let good = r#"<key value="A" direction="down"/><key value="A" direction="up"/>"#;
        let cases = [
            ("unknown key", r#"<key value="NOT_A_KEY" direction="down"/>"#.to_string()),
            ("unknown mouse", r#"<mousebutton button="extra" direction="down"/>"#.to_string()),
            ("unknown event", format!("{good}<mousemove x=\"3\" y=\"0\"/>")),
            ("unknown wheel", format!("{good}<mousewheel direction=\"up\"/>")),
            ("missing direction", r#"<key value="A"/>"#.to_string()),
            ("bad direction", r#"<key value="A" direction="sideways"/>"#.to_string()),
            ("negative delay", format!("{good}<delay milliseconds=\"-2\"/>")),
            ("overflow delay", format!("{good}<delay milliseconds=\"4294967296\"/>")),
            ("noninteger delay", format!("{good}<delay milliseconds=\"1.5\"/>")),
            ("missing delay", format!("{good}<delay/>")),
            ("unknown attribute", format!("{good}<delay milliseconds=\"3\" mystery=\"x\"/>")),
            ("nested content", format!("{good}<delay milliseconds=\"3\"><unexpected/></delay>")),
            ("foreign namespace", r#"<key xmlns="urn:other" value="A" direction="down"/><key value="A" direction="up"/>"#.to_string()),
            ("unbalanced down", r#"<key value="LCTRL" direction="down"/>"#.to_string()),
            ("unbalanced up", r#"<key value="A" direction="up"/>"#.to_string()),
            ("duplicate down", r#"<key value="A" direction="down"/><key value="A" direction="down"/><key value="A" direction="up"/>"#.to_string()),
            ("unbalanced mouse", r#"<mousebutton button="left" direction="down"/>"#.to_string()),
            ("duplicate mouse", r#"<mousebutton button="left" direction="down"/><mousebutton button="left" direction="down"/><mousebutton button="left" direction="up"/>"#.to_string()),
            ("empty", String::new()),
            ("only wait", r#"<delay milliseconds="25"/>"#.to_string()),
        ];
        for (label, steps) in cases {
            let macros = format!("{}<macro guid=\"b\" name=\"safe\"><keystroke><key value=\"B\"/></keystroke></macro>",
                sequence_xml("a", "Unsafe", "", &steps));
            let imported = import(&profile_xml(&macros, &[assignment("a", "G1", 1), assignment("b", "G2", 1)].concat(), "")).unwrap();
            assert_eq!(imported.skipped.len(), 1, "{label}: {:?}", imported.skipped);
            assert!(imported.skipped[0].contains("G1"), "{label}: {:?}", imported.skipped);
            assert!(key(&imported.dto, MBank::M1, "G1").is_none(), "{label}");
            assert_eq!(key(&imported.dto, MBank::M1, "G2").as_deref(), Some(r#""B""#), "{label}");
            assert!(imported.dto.macros.is_empty(), "{label}");
            from_dto(imported.dto).unwrap();
        }
    }

    #[test]
    fn rejects_unknown_repeat_modes_invalid_intervals_and_too_fast_loops() {
        let keys = r#"<key value="A" direction="down"/><key value="A" direction="up"/>"#;
        for attrs in [r#"repeatmode="burst""#, r#"repeatmode="pressed" repeatdelay="5""#,
            r#"repeatmode="toggle" repeatdelay="-1""#, r#"repeatmode="pressed" repeatdelay="abc""#,
            r#"repeatmode="pressed" repeatdelay="4294967296""#, r#"repeatmode="pressed" repeatcount="3""#,
            r#"repeatMode="pressed""#, r#"xmlns:r="urn:unexpected" r:repeatmode="pressed""#] {
            let imported = import(&profile_xml(&sequence_xml("a", "Unsafe", attrs, keys), &assignment("a", "G1", 1), "")).unwrap();
            assert_eq!(imported.skipped.len(), 1, "{attrs}");
            assert!(imported.dto.macros.is_empty(), "{attrs}");
            assert!(key(&imported.dto, MBank::M1, "G1").is_none(), "{attrs}");
        }
        let copy = r#"<macro guid="a" name="Copy" repeatmode="pressed"><hotkeys><do task="copy"/></hotkeys></macro>"#;
        let imported = import(&profile_xml(copy, &assignment("a", "G1", 1), "")).unwrap();
        assert_eq!(imported.skipped.len(), 1, "repetition of other command types must not be ignored");
        assert!(key(&imported.dto, MBank::M1, "G1").is_none());
    }

    #[test]
    fn malformed_keystrokes_and_duplicate_guids_are_not_guessed() {
        let cases = [
            r#"<macro guid="a" name="bad"><keystroke><key value="C"/><modifier value="W"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad"><keystroke><key value="C"/><unknown value="LCTRL"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad"><keystroke><key value="C"/><modifier value="LCTRL"/><modifier value="LCTRL"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad"><keystroke><key value="C"/></keystroke><keystroke><key value="B"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad">unexpected<keystroke><key value="C"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad"><keystroke><key xmlns="urn:other" value="C"/></keystroke></macro>"#,
            r#"<macro guid="a" name="bad"><keystroke><key value="C"/></keystroke></macro><macro guid="A" name="other"><keystroke><key value="B"/></keystroke></macro>"#,
        ];
        for macros in cases {
            let imported = import(&profile_xml(macros, &assignment("a", "G1", 1), "")).unwrap();
            assert_eq!(imported.skipped.len(), 1, "{macros}");
            assert!(key(&imported.dto, MBank::M1, "G1").is_none(), "{macros}");
            from_dto(imported.dto).unwrap();
        }
    }

    #[test]
    fn media_tasks_match_whole_names_and_unknown_functions_are_reported() {
        let macros = r#"
          <macro guid="a" name="Pause"><media><do task="playpause"/></media></macro>
          <macro guid="b" name="Next"><media><do task="nexttrack"/></media></macro>
          <macro guid="c" name="Previous"><media><do task="previoustrack"/></media></macro>
          <macro guid="d" name="Stop"><media><do task="stop"/></media></macro>
          <macro guid="e" name="Volume"><media><do task="volumeup"/></media></macro>
          <macro guid="f" name="Player"><function><do task="mediaplayer"/></function></macro>
          <macro guid="g" name="Unknown"><function><do task="nextdoor"/></function></macro>
          <macro guid="h" name="Ambiguous"><media><do task="nexttrack"/><do task="stop"/></media></macro>
        "#;
        let assigns = [('a', 1), ('b', 2), ('c', 3), ('d', 4), ('e', 5), ('f', 6), ('g', 7), ('h', 8)]
            .iter().map(|(guid, id)| assignment(&guid.to_string(), &format!("G{id}"), 1)).collect::<String>();
        let imported = import(&profile_xml(macros, &assigns, "")).unwrap();
        for (id, expected) in [("G1", "MediaPlayPause"), ("G2", "MediaNext"), ("G3", "MediaPrevious"),
            ("G4", "MediaStop"), ("G5", "VolumeUp")] {
            assert_eq!(key(&imported.dto, MBank::M1, id), Some(format!("\"{expected}\"")));
        }
        for id in ["G6", "G7", "G8"] {
            assert!(key(&imported.dto, MBank::M1, id).is_none());
            assert!(imported.skipped.iter().any(|s| s.contains(id)), "{id}: {:?}", imported.skipped);
        }
        assert_eq!(imported.skipped.len(), 3);
        from_dto(imported.dto).unwrap();
    }

    #[test]
    fn reports_profile_level_lua_and_unknown_bindings() {
        let macros = r#"<macro guid="a" name="A"><keystroke><key value="A"/></keystroke></macro>"#;
        let xml = profile_xml(macros, &assignment("a", "G99", 1), "<script>function OnEvent() end</script>");
        let imported = import(&xml).unwrap();
        assert_eq!(imported.skipped.len(), 2, "{:?}", imported.skipped);
        assert!(imported.skipped.iter().any(|s| s.contains("G99")));
        assert!(imported.skipped.iter().any(|s| s.contains("Lua")));
        from_dto(imported.dto).unwrap();
        let empty = import(&profile_xml("", "", "<script>  <!-- empty --> </script>")).unwrap();
        assert!(empty.skipped.is_empty());
        let external = import(&profile_xml("", "", r#"<script path="other.lua"/>"#)).unwrap();
        assert!(external.skipped.iter().any(|s| s.contains("Lua")));
    }

    #[test]
    fn imports_g13_bindings_colours_and_target() {
        let r = import(SAMPLE).unwrap();
        let d = &r.dto;
        assert_eq!(d.name, "WARDOGS");
        assert_eq!(d.rules, vec![MatchRule::exe("wardogs.exe")]);
        // backup="true" ignored: G1/M1 is ESCAPE, not "1"
        assert_eq!(key(d, MBank::M1, "G1").as_deref(), Some(r#""Escape""#));
        assert_eq!(key(d, MBank::M2, "G1").as_deref(), Some(r#""1""#));
        // physical positions kept as is: LGS "W" = our W (AZERTY Z)
        assert_eq!(key(d, MBank::M1, "G4").as_deref(), Some(r#""W""#));
        assert_eq!(key(d, MBank::M1, "STICK_UP").as_deref(), Some(r#""W""#));
        assert_eq!(key(d, MBank::M3, "STICK_LEFT").as_deref(), Some(r#""Numpad2""#));
        assert_eq!(key(d, MBank::M1, "G25").as_deref(), Some(r#"{"mouse":"Left"}"#));
        assert_eq!(key(d, MBank::M2, "G2").as_deref(), Some(r#""LeftCtrl+C""#));
        assert_eq!(key(d, MBank::M1, "G5").as_deref(), Some(r#""LeftCtrl+LeftShift+Escape""#));
        // keyboard (G110…) assignments are not G13 ones; G6 (multikey) is skipped
        assert_eq!(d.banks.get(MBank::M1).keys.len(), 5);
        assert_eq!(d.banks.get(MBank::M1).color.as_deref(), Some("#FF8000"));
        assert_eq!(d.banks.get(MBank::M3).color, None);
        // reported, not guessed
        assert_eq!(r.skipped.len(), 2, "{:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.contains("undo")));
        assert!(r.skipped.iter().any(|s| s.contains("multikey")));
        // and it is a valid profile for the engine
        from_dto(r.dto.clone()).unwrap();
    }

    #[test]
    fn export_then_import_roundtrips() {
        let original = import(SAMPLE).unwrap().dto;
        let (xml, skipped) = export(&original);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(xml.contains("devicecategory=\"Logitech.Gaming.LeftHandedController\""));
        assert!(xml.contains("contextid=\"G26\""), "stick up exported as G26");
        let back = import(&xml).unwrap();
        assert!(back.skipped.is_empty(), "{:?}", back.skipped);
        for bank in MBank::ALL {
            assert_eq!(back.dto.banks.get(bank).color, original.banks.get(bank).color);
            let a: Vec<_> = original.banks.get(bank).keys.iter().map(|(k, v)| (k.clone(), serde_json::to_string(v).unwrap())).collect();
            let b: Vec<_> = back.dto.banks.get(bank).keys.iter().map(|(k, v)| (k.clone(), serde_json::to_string(v).unwrap())).collect();
            assert_eq!(a, b, "{bank:?}");
        }
        assert_eq!(back.dto.rules, original.rules);
    }

    #[test]
    fn export_reports_what_lgs_cannot_hold() {
        let mut d = FileDto { name: "x & <y>".into(), ..FileDto::default() };
        d.banks.m1.keys.insert("G1".into(), ActionDto::Wheel { wheel: 1, horizontal: false });
        d.banks.m1.keys.insert("G2".into(), ActionDto::Macro { name: "spam".into() });
        d.banks.m1.keys.insert("G3".into(), ActionDto::Chord("Ctrl+C".into()));
        let (xml, skipped) = export(&d);
        assert_eq!(skipped.len(), 2);
        assert!(xml.contains("name=\"x &amp; &lt;y&gt;\""));
        roxmltree::Document::parse(&xml).unwrap();
    }

    /// Manual check on a real LGS export: `G13_LGS_SAMPLE=path cargo test -- --ignored lgs`
    #[test]
    #[ignore]
    fn import_real_file() {
        let path = std::env::var("G13_LGS_SAMPLE").expect("set G13_LGS_SAMPLE");
        let r = import(&std::fs::read_to_string(path).unwrap()).unwrap();
        for bank in MBank::ALL {
            eprintln!("{bank:?} {:?} {:?}", r.dto.banks.get(bank).color, r.dto.banks.get(bank).keys);
        }
        eprintln!("macros: {:#?}", r.dto.macros);
        eprintln!("skipped: {:#?}", r.skipped);
        from_dto(r.dto).unwrap();
    }
}
