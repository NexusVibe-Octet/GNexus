# G Nexus — consignes du projet

Logiciel Rust (Tauri 2) qui remplace Logitech Gaming Software et G HUB pour trois appareils :
clavier **G13**, souris **G502 X PLUS** (HID++ 2.0), casque **PRO X Wireless** (HID++ 2.0, dongle 046D:0ABA).
La version Windows est terminée et validée par l'utilisateur ; le **portage Linux** (CachyOS, Wayland,
PipeWire ; Bazzite par l'AppImage) est fait : voir [docs/LINUX_PORT.md](docs/LINUX_PORT.md).
Linux se distribue en paquet Arch (`deploy/linux/arch`) et en AppImage (`.github/workflows/linux-appimage.yml`).

L'utilisateur parle français : répondre en français. Il est en clavier **AZERTY**.

## Règles à ne jamais enfreindre

- `g13-core` reste sans aucun code dépendant du système.
- **Aucune touche ne doit rester enfoncée** : déconnexion, changement de profil et arrêt relâchent tout.
- Les `KeyCode` sont des **positions physiques** (noms US) : `W` produit `Z` en AZERTY. Même chose sous Linux
  (codes evdev = positions).
- Tout code propre à un système passe par la couche `platform` avec `#[cfg]` ; **la version Windows doit
  continuer à compiler et à fonctionner à l'identique** (ne rien casser côté `cfg(windows)`).
- Chaque fil d'appareil est l'unique propriétaire de son handle (G13, souris, casque).
- Mettre à jour `docs/ARCHITECTURE.md` après chaque fonctionnalité.
- Ne pas modifier `src/main.rs` à la racine (ancienne CLI).
- `wardogs.xml` (export LGS personnel) ne doit jamais être commité.
- Messages de commit en français, terminés par la ligne `Co-Authored-By` demandée par Claude Code.
- Le nom affiché est « G Nexus » ; les chemins internes gardent « G13 Studio » / `org.g13.studio` /
  `g13d` (compatibilité des installations existantes).

## Repères

- Espace de travail : `crates/g13-core`, `g13-daemon` (moteur, appareils, `platform/{windows,linux}`),
  `g13-gui` (Tauri : `src/main.rs`, page dans `ui/`), `logi-hidpp`, `g13-dsp` (égaliseur, sans OS),
  `g13-apo` (effet audio Windows uniquement).
- Tests : `cargo test --workspace` ; lint : `cargo clippy --workspace --all-targets`.
- Interface sans Tauri : `ui/dev-mock.js` simule le back-end quand `ui/index.html` est ouvert dans un navigateur.
- Traductions : `ui/i18n.js` (clé française → [en, de, it, es]) et `crates/g13-gui/src/i18n.rs`.
