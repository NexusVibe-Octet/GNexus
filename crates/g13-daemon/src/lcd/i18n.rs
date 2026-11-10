//! Texts of the LCD applets in the interface language. French is the source
//! text (as in the GUI's `ui/i18n.js`); the GUI tells the LCD which language
//! the page is shown in ([`crate::runtime::Runtime::set_lcd_language`]).
//!
//! Only letters the LCD font can draw: ASCII and the accents of
//! `g13_core::lcd` (no `ß`, for instance).

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    Fr,
    En,
    De,
    It,
    Es,
}

impl Lang {
    const ALL: [Lang; 5] = [Lang::Fr, Lang::En, Lang::De, Lang::It, Lang::Es];

    /// `"fr"`, `"en"`, `"de"`, `"it"`, `"es"` (as in the GUI).
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "fr" => Lang::Fr,
            "en" => Lang::En,
            "de" => Lang::De,
            "it" => Lang::It,
            "es" => Lang::Es,
            _ => return None,
        })
    }
}

/// The language shared between the GUI side and the LCD (engine thread).
#[derive(Debug, Clone, Default)]
pub struct LangCell(Arc<AtomicU8>);

impl LangCell {
    pub fn get(&self) -> Lang {
        Lang::ALL.get(self.0.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
    }

    pub fn set(&self, lang: Lang) {
        self.0.store(lang as u8, Ordering::Relaxed);
    }
}

/// French text → [en, de, it, es].
const TR: &[(&str, [&str; 4])] = &[
    // applet names (banner when an applet is selected)
    ("Profil", ["Profile", "Profil", "Profilo", "Perfil"]),
    ("Profils", ["Profiles", "Profile", "Profili", "Perfiles"]),
    ("Média", ["Media", "Medien", "Media", "Multimedia"]),
    ("Horloge", ["Clock", "Uhr", "Orologio", "Reloj"]),
    ("Système", ["System", "System", "Sistema", "Sistema"]),
    // Profil
    ("PROFIL", ["PROFILE", "PROFIL", "PROFILO", "PERFIL"]),
    ("PROFIL - FIXE", ["PROFILE - PINNED", "PROFIL - FEST", "PROFILO - FISSO", "PERFIL - FIJO"]),
    ("Actif partout", ["Active everywhere", "Überall aktiv", "Attivo ovunque", "Activo en todas partes"]),
    ("Profil par défaut", ["Default profile", "Standardprofil", "Profilo predefinito", "Perfil predeterminado"]),
    // Profils
    ("PROFILS", ["PROFILES", "PROFILE", "PROFILI", "PERFILES"]),
    ("FIXE", ["PINNED", "FEST", "FISSO", "FIJO"]),
    ("Auto (selon le jeu)", ["Auto (per game)", "Auto (je nach Spiel)", "Auto (in base al gioco)", "Auto (según el juego)"]),
    ("HAUT", ["UP", "AUF", "SU", "ARRIBA"]),
    ("BAS", ["DOWN", "AB", "GIÙ", "ABAJO"]),
    // Média
    ("MEDIA", ["MEDIA", "MEDIEN", "MEDIA", "MEDIOS"]),
    ("Aucun média en cours", ["Nothing playing", "Keine Wiedergabe", "Nessuna riproduzione", "Nada en reproducción"]),
    ("> LECTURE", ["> PLAYING", "> WIEDERGABE", "> RIPRODUZIONE", "> REPRODUCIENDO"]),
    ("|| PAUSE", ["|| PAUSED", "|| PAUSE", "|| PAUSA", "|| PAUSA"]),
    ("[] ARRET", ["[] STOPPED", "[] GESTOPPT", "[] FERMO", "[] DETENIDO"]),
    // Système
    ("SYSTEME", ["SYSTEM", "SYSTEM", "SISTEMA", "SISTEMA"]),
    ("SOURIS", ["MOUSE", "MAUS", "MOUSE", "RATÓN"]),
    ("Go", ["GB", "GB", "GB", "GB"]),
];

/// `fr` in `lang` (`fr` itself when French or missing from the table).
pub fn tr(lang: Lang, fr: &'static str) -> &'static str {
    match lang {
        Lang::Fr => fr,
        _ => TR.iter().find(|(k, _)| *k == fr).map_or(fr, |(_, t)| t[lang as usize - 1]),
    }
}

const DAYS: [[&str; 7]; 5] = [
    ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"],
    ["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"],
    ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
];

const MONTHS: [[&str; 12]; 5] = [
    [
        "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre",
        "décembre",
    ],
    [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
        "December",
    ],
    [
        "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November",
        "Dezember",
    ],
    [
        "gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre",
        "novembre", "dicembre",
    ],
    [
        "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre",
        "diciembre",
    ],
];

/// "lundi 9 novembre 2026", "Montag 9. November 2026"…; without the year
/// when `year` is false (the long dates do not fit on the screen).
/// `weekday` 0 = Monday, `month0` 0 = January.
pub fn date(lang: Lang, weekday: usize, day: u32, month0: usize, year: i32, with_year: bool) -> String {
    let (d, m) = (DAYS[lang as usize][weekday % 7], MONTHS[lang as usize][month0 % 12]);
    let dot = if lang == Lang::De { "." } else { "" };
    if with_year { format!("{d} {day}{dot} {m} {year}") } else { format!("{d} {day}{dot} {m}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g13_core::lcd::drawable;

    #[test]
    fn every_text_can_be_drawn() {
        let all = TR
            .iter()
            .flat_map(|(fr, t)| std::iter::once(*fr).chain(t.iter().copied()))
            .chain(DAYS.iter().flatten().copied())
            .chain(MONTHS.iter().flatten().copied());
        for text in all {
            assert!(text.chars().all(drawable), "« {text} » has a letter the LCD font cannot draw");
        }
    }

    #[test]
    fn translates_and_falls_back_to_french() {
        assert_eq!(tr(Lang::En, "Horloge"), "Clock");
        assert_eq!(tr(Lang::Fr, "Horloge"), "Horloge");
        assert_eq!(tr(Lang::De, "pas dans la table"), "pas dans la table");
        assert_eq!(date(Lang::De, 0, 9, 10, 2026, true), "Montag 9. November 2026");
        assert_eq!(date(Lang::Fr, 2, 30, 8, 2026, false), "mercredi 30 septembre");
    }

    #[test]
    fn the_cell_round_trips() {
        let c = LangCell::default();
        assert_eq!(c.get(), Lang::Fr);
        for l in Lang::ALL {
            c.set(l);
            assert_eq!(c.get(), l);
        }
        assert_eq!(Lang::from_code("es"), Some(Lang::Es));
        assert_eq!(Lang::from_code("xx"), None);
    }
}
