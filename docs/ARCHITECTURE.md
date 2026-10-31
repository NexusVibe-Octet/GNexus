# Architecture — `g13-daemon` et G13 Studio

## Vue d'ensemble

```
                        ┌──────────── runtime (g13d ou G13 Studio) ───────────────────┐
  G13 ◀══ USB/HID ══▶   │  device/hid.rs  (thread "g13-hid", propriétaire du HidDevice)│
                        │    read 0x01 ─▶ tracker (diff bitmask) ─▶ DeviceEvent ──┐   │
                        │    ◀── DeviceCommand (LEDs 0x05, RGB 0x07, LCD 0x03) ◀─┐│   │
                        │                                                        ││   │
                        │  engine/  (thread "g13-engine")                        ││   │
                        │    M-banks · bindings · macros · stick* ───────────────┘│   │
                        │       │ InputInjector          │ UiEvent ◀──────────────┘   │
                        │       ▼                        ▼                            │
                        │  platform/{windows,linux}   lcd/* · ipc/*                   │
                        │  profiles/ (appelé à chaque tick du moteur) ─▶ set_profile  │
                        └─────────────────────────────────────────────────────────────┘
                                                         (* = étapes suivantes)
```

Principes :
- **Un seul propriétaire du périphérique** : le thread HID. Tout le monde lui parle par
  canal (`DeviceCommand`), donc LEDs, RGB et trames LCD ne se marchent jamais dessus.
- **Événements par fronts** : le moteur ne voit que des transitions `Pressed/Released`,
  jamais l'état brut répété.
- **Jamais de touche bloquée** : chaque G-key mémorise ce qu'elle a enfoncé (`Held`) et le
  relâchement défait exactement cela, même après un changement de banque/profil. Débranchement,
  changement de profil et arrêt relâchent tout et stoppent les macros.
- **OS derrière des traits** : le moteur ne connaît que `dyn InputInjector`,
  `dyn ForegroundWatcher` et `dyn MediaProvider`.

## Arborescence

```
Cargo.toml                    workspace + CLI d'exploration (src/main.rs, inchangé)
crates/
  g13-core/                   protocole pur, sans OS (existant)
    keys.rs                   + G13Key::ALL / bit_index() (index compact 0..34) /
                                report_bit() (position réelle dans le rapport) / id() / from_id()
    input.rs                  + raw_bits() ; key_mask() regroupe les bits bruts en 34 bits
                                compacts et ignore les bits d'état ; pressed_keys()
  logi-hidpp/                 protocole HID++ 2.0 pur, sans OS (souris Logitech : G502 X PLUS…)
    lib.rs                    Request / Message (rapports longs 0x11, erreurs 2.0 et 1.0, sw_id)
    features.rs               ids des features, décodage des réponses en lecture
    onboard.rs                annuaire et profils de la mémoire interne, CRC, LED, boutons
  g13-daemon/                 bibliothèque + binaire `g13d` (sans interface)
    src/main.rs               arguments (--profiles, --no-auto…), Ctrl+C (+ SIGTERM / SIGHUP sous Unix)
    src/bin/hidpp_probe.rs    `hidpp-probe` : exploration HID++ en lecture seule + copie de la
                                mémoire interne des souris
    src/bin/hidpp_lab.rs      `hidpp-lab` : essais d'écriture encadrés (profil d'essai, restore)
    src/mouse/link.rs         HID++ sur hidapi : requête / réponse, lecture et écriture de secteurs
    src/runtime.rs            Runtime : threads HID + moteur (+ ProfileManager), flux UiEvent,
                                requêtes (RescanProfiles) — partagé par g13d et G13 Studio
    src/keycode.rs            KeyCode (positions physiques) + tables scancode Win32 / evdev
    src/device/
      mod.rs                  DeviceEvent / DeviceCommand
      tracker.rs              diff de rapports → fronts, hystérésis du stick
      hid.rs                  thread HID, lecture Interrupt|Poll, hotplug, exécution commandes
    src/engine/
      mod.rs                  Engine : dispatch, M1/M2/M3, MR, BD, L1–L4, UiEvent
      action.rs               Action, Macro, MacroStep, MacroMode
      bindings.rs             MBank, BankBindings (25 touches), Profile
      macros.rs               lecteur de macros (thread par macro, annulation immédiate)
      demo.rs                 profil de démo (source de demo.toml, secours en mémoire)
      tests.rs                tests du moteur (injecteur enregistreur)
    src/profiles/
      format.rs               format TOML : DTO serde ↔ Profile / BankBindings / Action / Macro
      rules.rs                MatchRule (exe / titre) et sélection
      store.rs                dossier de profils (crate directories), génération de demo.toml,
                                relecture des fichiers modifiés
      manager.rs              ProfileManager : bascule auto (4 Hz) + rechargement à chaud (1 s)
      lgs.rs                  import / export des profils XML de Logitech Gaming Software
      tests.rs                aller-retour TOML, erreurs, règles, store, bascule
    src/lcd/
      mod.rs                  LcdManager (sélection L1–L4, bandeau, thread média, rendu ≤ 4 Hz)
      applets.rs              Profil · Horloge · Média · Système
    src/platform/
      mod.rs                  traits + fabriques default_*()
      mock.rs                 RecordingInjector (--dry-run, tests), watchers nuls
      windows/                injector.rs (SendInput) · foreground.rs · media.rs
      linux/                  injector.rs + gamepad.rs (uinput) · recorder.rs (evdev) · foreground.rs (KWin) · media.rs (MPRIS) · audio.rs (PipeWire) · access.rs (règles udev)
  g13-gui/                    G13 Studio (Tauri 2), binaire `g13-studio`
    src/main.rs               fenêtre, icône de notification, autostart, instance unique,
                                commandes Tauri, relais UiEvent → événement "g13"
    src/profiles_io.rs        liste / lecture / écriture / création / suppression des profils
    src/conflicts.rs          détection / fermeture de LGS et G HUB (sysinfo)
    src/settings.rs           studio.json (démarrer réduit, fermer = réduire, langue, clavier)
    src/backup.rs             sauvegarde complète .g13backup (export / import / réinitialisation)
    src/i18n.rs               textes côté Rust (menu de l'icône, boîtes de fichiers) traduits
    ui/                       interface HTML/CSS/JS sans étape de build
      index.html · style.css  structure et thème (maquette sombre)
      device.js               dessin du G13 (espace 760×820) et touches cliquables
      i18n.js                 traductions fr / en / de / it / es (clé = texte français)
      catalog.js              catalogue des actions, 5 dispositions de clavier ↔ noms KeyCode
      app.js                  état, sélecteur d'action, macros, règles, paramètres
      mouse.js · mouse-device.js  page Souris (réglages en direct) et plan de la souris (boutons)
      dev-mock.js             faux back-end pour développer l'UI dans un navigateur
    icons/                    icônes (make_icon.ps1 les régénère)
.cargo/config.toml            dossier de build hors du chemin à espaces (voir « Build »)
deploy/linux/                 règles udev (G13, G502 X PLUS, casque, uinput ; uaccess ou groupe input), uinput.conf,
                              gnexus.desktop ; arch/ : PKGBUILD du paquet `gnexus` (Arch / CachyOS)
docs/ARCHITECTURE.md
```

## Couche plateforme

| Trait | Windows (fait) | Linux / CachyOS (en cours) |
|---|---|---|
| `InputInjector` | `SendInput` en **scancodes** (+ VK pour touches média/Pause), `dwExtraInfo = "G13D"` | **fait** : `/dev/uinput` via `evdev`, deux périphériques « G Nexus virtual keyboard » / « G Nexus virtual mouse » (codes `KeyCode::evdev()` = positions, molette basse + haute résolution). Une touche encore tenue est relâchée par le noyau quand le périphérique disparaît (même après SIGKILL) |
| `InputInjector::gamepad` / `gamepad_unplug` | manette Xbox 360 virtuelle via ViGEmBus (`platform/windows/gamepad.rs`, crate `vigem-client`) | **fait** : manette Xbox 360 uinput (`platform/linux/gamepad.rs`), copie de ce que le pilote `xpad` expose (`045E:028E`, Y inversé, gâchettes `ABS_Z/RZ`, croix sur `HAT0`) : SDL / Steam / Proton la voient comme une vraie 360. `virtual_gamepad_available` = `/dev/uinput` accessible en écriture |
| `ForegroundWatcher` | `GetForegroundWindow` → PID → `QueryFullProcessImageNameW` + titre (implémenté) | **fait pour KDE Plasma** : script KWin chargé par D-Bus (`org.kde.KWin /Scripting`), qui rappelle G Nexus à son nom D-Bus unique (`zbus`, objet `/org/g13/studio/Focus`) à chaque changement de fenêtre active, de titre ou de liste ; un seul suivi par processus, script retiré par `platform::shutdown()` (appelé par `Runtime::stop`). Nom : `.exe` Windows de la ligne de commande (Wine / Proton : mêmes règles que sous Windows), sinon `/proc/<pid>/exe`. Autres bureaux : `Unsupported`, changement automatique désactivé |
| `MediaProvider` | contrôle par touches média (fait), métadonnées `Windows.Media.Control` (à faire) | **fait** : MPRIS via `zbus` (`platform/linux/media.rs`) : lecteur en lecture, sinon le dernier montré, sinon en pause ; propriétés lues sans cache (`Position`), commandes envoyées à ce lecteur |

Les noms de `KeyCode` sont des **positions physiques** (libellés US), comme les codes HID/evdev :
`W` produit `Z` en AZERTY. C'est ce qu'attendent les jeux et c'est identique sous Linux.

## Flux d'une touche

1. `hid.rs` lit 8 octets (`read_timeout` 5 ms, ou `GetInputReport` avec `--poll`).
2. `G13InputReport::key_mask()` convertit les positions réelles (`report_bit`, non contiguës) en
   masque compact (`bit_index`, G1–G25 = 0..=24) et jette les bits d'état ; `tracker.rs` fait
   `now ^ prev` sur ce masque → relâchements puis appuis, par ordre d'index.
3. `Engine::on_key` :
   - **G1–G24, G25** → action de la banque active : `Key` (accord tenu), `MouseButton` (tenu),
     `Wheel` (à l'appui), `Macro` (`Once` / `RepeatWhileHeld` / `Toggle`), `Pad` (bouton de manette, tenu).
   - **M1/M2/M3** → banque + LEDs (0x05) + couleur de banque (0x07).
   - **MR** → mode enregistrement + LED (capture à venir).
   - **BD** → rétroéclairage on/off (+ `UiEvent::DisplayPower` pour couper le LCD).
   - **L1–L4** → `UiEvent::LcdButton` pour le gestionnaire d'applets.

**Latence** (mesurée en jeu, du rapport HID lu à `SendInput`) :
- sous Windows, le processus passe en **priorité haute** au démarrage (`platform::raise_priority`) :
  un jeu qui sature le processeur faisait attendre les fils du G13 en priorité normale (médiane 6 ms,
  plus de 60 ms une fois sur dix, jusqu'à 0,7 s) ; en priorité haute : médiane 0,3 ms, 9 sur 10 sous
  0,5 ms. G Nexus utilise bien moins de 1 % du processeur ;
- rien de lent sur le fil du moteur : l'application au premier plan est demandée par un fil à part
  (`BackgroundWatcher`, « g13-focus », toutes les 100 ms) et le titre de fenêtre est lu par
  `InternalGetWindowText` (`GetWindowTextW` attendait le jeu, jusqu'à 285 ms) ;
- les images LCD (~24 ms d'envoi) partent par un second handle sur un fil « g13-lcd » (la plus récente
  seulement) : elles ne retardent plus la lecture des touches.

## G13 Studio (interface)

Un seul processus, comme LGS : la fenêtre, l'icône de notification et le `Runtime` du daemon,
qui continue de piloter le G13 quand la fenêtre est cachée. `g13d` reste le mode sans interface
(service Linux à terme). Les deux ne doivent pas tourner en même temps (un seul propriétaire du G13) ;
l'application refuse elle-même une deuxième instance (plugin single-instance).

- **Modèle édité = le fichier de profil.** L'UI lit/écrit `FileDto` (même structure que le TOML,
  en JSON) ; chaque enregistrement passe par `from_dto` (même validation que le daemon), puis le
  fichier est réécrit (touches triées G1→G25) et `Request::RescanProfiles` applique le changement
  aussitôt. Les commentaires d'un fichier écrit à la main sont perdus s'il est modifié depuis l'UI.
- **Temps réel** : `UiEvent` → thread `g13-ui-events` → événement Tauri `g13`
  (`key`, `bank`, `profile`, `connected`, `record`, `display`) : la touche pressée s'allume à l'écran,
  la banque affichée suit M1/M2/M3, l'état « G13 connecté / non détecté » est à jour.
- **Écran** (d'après les maquettes, corrigées pour le vrai G13) :
  - à gauche MODE STICK (clavier / manette) et MENUS M1/M2/M3 — les « layers 0–7 »
    des maquettes n'existent pas sur le G13 ; à droite PROFILS numérotés (numéro stable `number`
    dans le fichier, nom au survol et dans l'en-tête, « + » pour en créer) ;
  - dessin vectoriel fidèle (`device.js`) : cuvette en sablier (rangées 1–2 presque droites,
    rangées 3–4 en biais), touches de bord trapézoïdales (G1, G7, G8, G14, G15, G19, G20, G22),
    G16–G18 sous G10–G12, G21 sous G17 ; LCD + BD + L1–L4, bande M ; proportions de la photo
    (hauteur ≈ 1,45 × largeur, repose-poignet ≈ le tiers inférieur) ; le module pouce est une carte
    séparée à droite, reliée au corps : anneau de 4 directions assignables autour du clic G25,
    G23 et G24, zone morte ;
  - COULEUR + luminosité par menu : envoyée au G13 (rétroéclairage touches + écran) et reproduite
    dans le logiciel (roue de couleurs au-dessus du bouton COULEUR, libellés des touches et écran LCD
    teintés) ; sa teinte devient aussi la couleur d'accent de toute l'interface (`--accent`). Le
    choix se fait dans une fenêtre centrée : 6 couleurs de base, carré teinte/saturation, code hexa ;
    la couleur s'essaie en direct sur le G13, « Annuler » (ou Échap / clic à côté) rétablit l'ancienne ;
  - sélecteur d'action à onglets (Clavier, Touches, Média, Souris, Manette, Macros, modificateurs),
    « Paramètres avancés » (applications associées, éditeur de macros), écran « G13 non détecté » ;
  - glisser-déposer d'une touche sur une autre (`Device.dragKeys`, `moveKey` dans app.js) : au-delà de
    6 px la pression devient un glissement (pas de clic), la touche visée s'allume ; au relâchement, son
    action remplace celle de la cible et la touche de départ devient vierge (menu affiché seulement).
- **Aperçu sur le G13** : le profil affiché est envoyé au runtime (`Request::Preview`) ; tant que la
  fenêtre de G13 Studio a le focus (exe au premier plan = le nôtre), c'est lui qui est actif sur le
  G13, donc couleur et touches s'essaient en direct. Ailleurs, la bascule automatique reprend.
- **Paramètres** (roue dentée) : lancer au démarrage de Windows (plugin autostart, clé `Run` ;
  lancé avec `--autostart`), rester réduit dans la zone de notification dans ce cas, fermer la
  fenêtre = la cacher. Quitter : menu de l'icône de notification (relâche tout avant de sortir).
- **Libellés** : le catalogue affiche ce que produit le clavier choisi (AZERTY : rangée des chiffres
  `&` `é` `"`…, le chiffre en petit) mais enregistre les noms `KeyCode` (positions physiques),
  donc un profil reste le même quelle que soit la disposition affichée. Au démarrage, l'UI compare
  son catalogue à `key_names`. Un nom trop long pour sa touche (macro…) est coupé avec « … » ; le
  nom complet s'affiche au survol.
- **À propos** (Paramètres, 1.0.0) : nom, version (`app_version` = `tauri.conf.json`), présentation,
  bouton GitHub, bouton Ko-fi (image officielle copiée dans `ui/kofi.png` : rien n'est chargé depuis
  Ko-fi), crédit design (JaspMakes), mention « non affilié à Logitech ». Les liens passent par
  `open_link(which)` : les adresses sont fixées dans `LINKS` (main.rs), la page ne nomme que l'une
  d'elles (`github`, `kofi`, `jaspmakes`) et le navigateur par défaut s'ouvre (plugin opener).
- **Langues** (Paramètres) : français, anglais, allemand, italien, espagnol, ou « langue du système »
  (`navigator.language`). Système : `OS` / `ON_LINUX` (i18n.js, d'après le navigateur intégré :
  WebKitGTK sous Linux, WebView2 sous Windows ; `?os=linux` avec dev-mock) et classe `os-linux` /
  `os-windows` sur `<html>` ; les éléments `win-only` / `linux-only` ne s'affichent que sur leur
  système (textes ViGEmBus, droits administrateur, « démarrage de Windows »…).
  `ui/i18n.js` : la clé est le texte français, `t("Profil « {name} »",
  { name })` ; le HTML statique est traduit une fois au démarrage (`translateDom` : nœuds texte,
  `title`, `placeholder`, éléments `data-i18n-html`). Changer de langue recharge les deux fenêtres
  (événement `language-changed`). Le clavier du sélecteur suit la langue (fr → AZERTY, en → QWERTY US,
  de → QWERTZ, it / es → QWERTY italien / espagnol), ou un choix explicite ; les noms des touches
  (Échap / Esc / Entf / Canc / Supr…) suivent la langue. Côté Rust, seuls le menu de l'icône et les
  boîtes de fichiers sont traduits (`i18n.rs`, langue signalée par la page via `ui_language`) ; les
  messages d'erreur et l'écran LCD du G13 restent en français.
- **Effacer les touches** (gomme de l'en-tête) : retire les actions des touches G et des directions
  du stick du menu affiché, ou de M1 à M3 ; couleurs, macros, applications et mode du stick restent.
- **Configuration complète** (Paramètres) : EXPORTER écrit tous les profils (texte TOML tel quel) et
  les préférences dans un fichier `.g13backup` (JSON) ; IMPORTER le valide entièrement (format,
  identifiants sûrs, chaque profil passé par `from_toml`) puis remplace profils et préférences ;
  RÉINITIALISER ne laisse que le profil de démo et les préférences par défaut (la langue et
  « Lancer au démarrage de Windows » sont conservés). Import et réinitialisation enregistrent
  d'abord la configuration actuelle dans `%APPDATA%\g13d\config\backups\` : rien n'est perdu.
- **Mémoire du G13** : le G13 n'a pas de mémoire pour les affectations ; il envoie seulement l'état
  brut de ses touches et c'est le logiciel du PC qui les traduit (LGS faisait de même). G13 Studio
  lancé avec Windows et réduit dans la zone de notification en tient lieu.
- **Où s'applique le profil** : une barre sous l'en-tête dit si le profil affiché s'active dans des
  jeux, partout (par défaut ★), ou seulement pendant l'édition — avec « Utiliser par défaut » et
  « Associer à un jeu » dans ce dernier cas.
- **Associer une application** (Paramètres avancés › Applications) : applications ouvertes
  (`platform::running_apps`, fenêtres visibles façon Alt+Tab), récemment au premier plan
  (`ProfileManager::recent_apps`, 25 dernières) et « Parcourir… » (sélecteur de `.exe`) ; un clic
  associe l'exécutable au profil affiché et le retire des autres profils.
- **Tester partout** (même barre) : `Request::Pin` / `ProfileManager::set_pinned` force le profil
  affiché dans toutes les applications (règles, défaut et aperçu ignorés) jusqu'à « Arrêter le
  test » ; non mémorisé au redémarrage. L'applet Profil du LCD affiche alors « PROFIL - TEST ».
- Lancer : `cargo run -p g13-gui` (options `--dry-run`, `--poll`). Développer l'UI sans G13 :
  servir `crates/g13-gui/ui/` en HTTP, `dev-mock.js` remplace le back-end.

## Build (Windows, toolchain GNU)

`windres` (utilisé par tauri-build pour l'icône) ne supporte pas les espaces du chemin du projet :
`.cargo/config.toml` place le dossier de build dans `C:/Users/Octet/.cargo-targets/g13` (à adapter
sur une autre machine). WebView2 est requis (présent sur Windows 11). L'éditeur de liens GNU affiche
« .rsrc merge failure: multiple non-default manifests » : sans effet constaté à ce jour.

### Linux : AppImage

Construite sur Ubuntu 22.04 (`cargo tauri build --bundles appimage`, ou le workflow
`.github/workflows/linux-appimage.yml`) ; `crates/g13-gui/tauri.linux.conf.json` ne s'applique que sous
Linux (binaire `gnexus`, cible `appimage`). Détails et choix : `docs/LINUX_PORT.md`, étape 11.

- **Accès aux appareils** (`g13_daemon::platform::linux::access`) : `status()` teste l'ouverture des
  nœuds hidraw du G13, de la souris et du dongle (appareil USB parent `046D:C21C/C095/C547/0ABA`) et de
  `/dev/uinput`, détecte un conteneur et l'autorisation de RECORD ; `install(record)` écrit les règles
  de `deploy/linux` (embarquées) dans `/etc` par `pkexec`. Commandes Tauri `device_access`,
  `install_device_access` (rend `true` s'il faut redémarrer : moteur non démarré faute de uinput) et
  `restart_app`. Interface : bandeau `#access-banner` (vérifié toutes les 5 s), section
  *Paramètres › Général › Accès aux appareils* (`linux-only`).
- **Démarrage automatique** : `enable_autostart` met le chemin de `Exec=` entre guillemets (chemin
  de l'AppImage avec un espace) ; `quote_desktop_exec` est testé.

### Version installable

Depuis `crates/g13-gui` : `npx @tauri-apps/cli@2 build` → compilation `--release` puis installateur
NSIS dans `<target-dir>/release/bundle/nsis/G13 Studio_<version>_x64-setup.exe`. Installation par
utilisateur (`%LOCALAPPDATA%\G13 Studio`, sans droits administrateur), menu Démarrer, désinstallation
dans « Applications installées » ; les profils (`%APPDATA%\g13d\config\profiles`) ne sont pas touchés.

Comportement de la version release :
- pas de console ; journal dans `%APPDATA%\g13d\config\studio.log` (réécrit à chaque lancement) ;
- « Lancer au démarrage » : l'entrée est réenregistrée à chaque lancement pour pointer vers l'exe
  installé (une version de développement ne la modifie jamais) ;
- Logitech Gaming Software (`LCore.exe`) détecté toutes les 15 s : bandeau d'avertissement avec
  bouton pour le fermer (`conflicts.rs`) ; il faut aussi désactiver son démarrage automatique ou le
  désinstaller. G HUB ne gère pas le G13 et cohabite sans problème (vérifié) : il n'est pas signalé.

## Écran LCD

`g13-daemon/src/lcd/` — `LcdManager::tick` est appelé par le runtime à chaque tick du moteur, après
`ProfileManager::tick`. Il rend au plus 4 images/s et n'envoie `Engine::draw_lcd` (→
`DeviceCommand::DrawLcd` au thread HID + `UiEvent::LcdFrame` pour l'aperçu) que si l'image change.

Comme LGS : **BD** (bouton rond à gauche des L) fait défiler les applets ; **L1–L4** sont les
touches de l'applet affiché, libellées en bas de l'écran (`Applet::keys` / `on_key` → `LcdAction`).
Le rétroéclairage reste piloté par le bouton lumière matériel du G13 (géré par son firmware).

| Applet (ordre BD) | Contenu | L1 · L2 · L3 |
|---|---|---|
| **Profil** | « PROFIL · MENU Mx », nom du profil, jeu associé / « Profil par défaut » / « Actif partout » | — · — · PROFILS (va à la liste) |
| **Profils** | « Auto (selon le jeu) » puis les profils dans l'ordre de G13 Studio (`ProfileStore::ordered`), `*` = choix courant, `<` = actif | HAUT · BAS · OK |
| **Média** | état, source, titre et artiste défilants, position / durée — `MediaProvider` interrogé 1×/s par le thread `g13-media`, seulement quand l'applet est affiché | précédent · lecture/pause · suivant |
| **Horloge** | HH:MM en grand, secondes, date en français (`chrono`, heure locale) | — |
| **Système** | CPU et RAM en barres (`sysinfo`) | — |

Choisir un profil dans « Profils » le **fixe partout** (`ProfileManager::set_pinned`, le même
mécanisme que « Tester partout » de G13 Studio, synchronisé par `UiEvent::Pinned`) ; « Auto »
rétablit la bascule automatique. Un bandeau inversé de 1,5 s annonce le changement d'applet, de menu
ou de profil. Après une reconnexion, l'image est renvoyée.

- **Rendu** (`g13-core/src/lcd.rs`) : police 5×7 + accents français dessinés dans les deux lignes
  libres au-dessus des minuscules (é è ê à ç…), capitales accentuées → lettre de base, caractères
  non dessinables (emoji…) ignorés sans trou ; texte agrandi (`draw_text_scaled`), fenêtre défilante
  (`draw_text_window`), `fill_rect`, `invert_rect`, `draw_bar`.
- **Windows « en cours de lecture »** (`platform/windows/media.rs`) : WinRT
  `GlobalSystemMediaTransportControlsSessionManager` (crate `windows`, appels bloquants `.join()` sur
  le thread média) ; position extrapolée depuis `LastUpdatedTime` pendant la lecture.
- **G13 Studio** : l'écran du dessin affiche l'image réelle (canvas 160×43, `image-rendering:
  pixelated`), teintée de la couleur du menu.

## Overlay en jeu

Une touche G réservée (paramètres, G25 = clic du stick par défaut, « Désactivé » possible) ouvre un
panneau compact par-dessus le jeu pour reconfigurer à la volée.

- **Moteur** : `Engine::set_overlay_key` / `RuntimeOptions::overlay_key` / `Request::SetOverlayKey` :
  la touche n'exécute plus son affectation, elle émet `UiEvent::OverlayToggle`.
- **Fenêtre** `overlay` (Tauri) : sans bordure, transparente hors de la carte, toujours au premier
  plan, hors barre des tâches, créée cachée ; même page que la fenêtre principale en mode `#overlay`
  (CSS `html.overlay`). Placée sur l'écran de la souris, ≈ 780 px × 90 % de la hauteur, à gauche,
  au centre ou à droite (paramètre). G25, Échap, ✕ ou Alt+F4 la cachent ; Windows rend le focus au jeu.
- **Profil** : à l'ouverture, l'overlay reçoit le profil et le menu actifs sur le G13 (ceux du jeu)
  et `Request::Preview` les garde actifs pendant qu'il a le focus. Pastilles de profils en haut à
  droite ; « Associer à ce jeu » ajoute l'exe du jeu (lu au moment de l'ouverture) au profil affiché
  et le retire des autres profils.
- **Limite** : fonctionne par-dessus les jeux en fenêtré / plein écran fenêtré ; un jeu en plein
  écran exclusif est réduit par Windows tant que l'overlay est affiché.
- **Linux (Wayland)** : une fenêtre ne peut ni se placer ni rester au premier plan d'elle-même. Le
  titre devient « G Nexus — overlay [left|center|right] » (`foreground::overlay_title`) et le script
  KWin de la détection du premier plan (`foreground::ensure_started`) reconnaît cette fenêtre de son
  propre processus : `keepAbove`, hors barre des tâches / sélecteur, placée sur l'écran actif, activée.
  Hors KDE Plasma, l'overlay s'ouvre comme une fenêtre ordinaire.

Le fil qui relaie les `UiEvent` ne doit appeler `toggle_overlay` qu'**en dehors** du verrou du
statut (`toggle_overlay` le relit) — un appel sous verrou a bloqué ce fil pendant la mise au point.

**Clic du stick** : appuyer sur le stick le fait pencher. Les directions déclenchées dans les 150 ms
avant le clic sont relâchées au moment du clic, et aucune nouvelle direction ne démarre pendant le
clic ni dans les 150 ms qui suivent (`STICK_CLICK_GRACE`).

## Mode manette (stick analogique)

`[stick] mode = "gamepad"` : le stick du G13 devient le joystick d'une **manette Xbox 360 virtuelle**
(XInput), vue par les jeux comme une vraie manette. Pilote **ViGEmBus** requis (installé à part ;
G13 Studio prévient s'il manque). Les touches G peuvent aussi recevoir des boutons de manette
(onglet MANETTE du sélecteur, `G1 = { pad = "A" }` : A B X Y, LB RB, LT RT en appui complet,
Back Start, LS RS = clics des sticks, Up Down Left Right = croix), dans n'importe quel mode du stick.

- `engine/pad.rs` : `Pad` compte les entrées qui tiennent chaque bouton (deux touches sur A : A reste
  appuyé jusqu'au dernier relâchement) et calcule `GamepadState`. Zone morte **radiale**, remise à
  l'échelle (la sortie part de 0 au bord de la zone morte) ; `target = "left" | "right"` choisit le
  joystick piloté. Dans l'interface : GAUCHE / DROIT dans la carte du stick ; en passant en mode
  manette, la zone morte de 35 % (réglage clavier) devient 15 %.
- Calibration automatique : la position de repos est réapprise quand le stick reste immobile près
  du centre (±0x16) pendant 1,5 s (`SETTLE`) ; l'amplitude part de ±0x50 autour du centre et
  s'élargit à chaque position vue, pour que la déflexion maximale donne toujours ±32767.
- **Branchée seulement quand il le faut** : la manette virtuelle est branchée quand le G13 est
  connecté et que le profil actif l'utilise (stick en mode manette ou bouton de manette assigné), et
  débranchée sinon (changement de profil, débranchement du G13, arrêt). Un jeu ne voit donc pas de
  manette fantôme avec un profil clavier. Changement de profil, débranchement et arrêt relâchent
  tous les boutons avant de débrancher (invariant « aucune touche bloquée »).
- Si ViGEmBus est absent ou refuse, un seul avertissement (`UiEvent::GamepadError` → message dans
  G13 Studio) et pas de nouvel essai avant la prochaine connexion / bascule de profil, pour ne pas
  ralentir le thread moteur à chaque mouvement du stick.
- Vérifié sur la machine de dev : `cargo test -p g13-daemon vigem -- --ignored` fait apparaître
  « Contrôleur XBOX 360 pour Windows » dans le Gestionnaire de périphériques, puis le retire.

## Souris G502 X PLUS (HID++ 2.0) — étape 1 : exploration

Objectif : remplacer aussi G HUB pour la souris (puis le casque PRO X Wireless), dans G13 Studio et
sous Linux. Étape 1 faite le 2026-10-16 : lecture seule, aucune écriture dans la souris.

**Protocole.** HID++ 2.0 : la souris annonce des *features* (id 16 bits) à des index propres à
l'appareil ; Root (index 0) donne l'index d'une feature, FeatureSet les liste. Requête = rapport
long `0x11 | index appareil | index feature | fonction << 4 | sw_id | 16 octets`. Notre
`sw_id` (0x0D) distingue nos réponses de celles de G HUB : l'exploration a tourné **G HUB ouvert**
sans interférence. Derrière un récepteur, l'appareil est à l'index 1..6 ; câblé seul, 0xFF.

**Matériel relevé** (`cargo run -p g13-daemon --bin hidpp-probe`) :

| | |
|---|---|
| Récepteur | LIGHTSPEED `046D:C547`, interface 2, collection vendeur 0xFF00 usage 2 (HID++ long) |
| Souris | G502 X PLUS à l'index 1, HID++ 4.2, firmware MPM 27.00.B0016, bootloader BL1 42.00.B0016 |
| Features utiles | UnifiedBattery 0x1004, AdjustableDpi 0x2201 (100–25 600 par pas de 50), HiResWheel 0x2121, RgbEffects 0x8071, PerKeyLightingV2 0x8081, OnboardProfiles 0x8100, MouseButtonSpy 0x8110, ReportRate 0x8060 (125–1000 Hz) |
| Absent | ReprogControlsV4 (0x1B04) : les boutons ne se « déroutent » pas comme sur une souris bureautique ; en mode logiciel il faudra passer par la mémoire interne (profil hôte) et/ou MouseButtonSpy |
| Mémoire interne | 5 profils + 2 d'usine, 11 boutons, 16 secteurs de 255 octets (format profil 5, macros format 1) ; mode actuel « piloté par un logiciel » (G HUB) |
| Aussi trouvé | clavier MX Keys sur le récepteur Unifying `046D:C52B` (HID++ 4.5), casque PRO X Wireless `046D:0ABA` (pages 0xFF00 / 0xFF43) |

**Mémoire interne** (copiée dans `%APPDATA%\g13d\config\backups\hidpp-g502-x-plus-<date>\`,
un fichier par secteur, avant toute écriture future) :
- secteur 0 = annuaire : entrées de 4 octets `secteur (BE) | activé | 0xFF`, fin 0xFFFF ; profils
  1 et 2 activés, 3 à 5 désactivés ;
- secteurs 1–5 = profils (255 octets, CRC-16/CCITT-FALSE sur les 253 premiers, stocké en fin, BE) :
  octet 0 fréquence (1 = 1 ms), 1 palier DPI par défaut, 2 palier « shift », 3–12 cinq DPI (LE :
  800/1200/1600/2400/3200), 32–75 boutons (11 × 4 octets), 96–139 les mêmes en G-shift, 160–207
  nom UTF-16 (vide ici), 208–218 et 219–229 effets LED (logo, côté) ;
- bouton = 4 octets : `80 01 mm mm` bouton souris, `80 02 mod touche` raccourci clavier (usage
  HID), `80 03 xx xx` touche multimédia, `90 nn` fonction (DPI ±, DPI shift, G-shift, profil…),
  `FF FF FF FF` désactivé ; le profil 2 de l'utilisateur a des raccourcis Ctrl+… en G-shift ;
- secteurs 0x0100+ = ROM d'usine (lecture seule) : 0x0101 est le profil d'usine (sans CRC), 0x0100
  n'est pas un profil.

### Étape 2 : écriture dans la mémoire (validée le 2026-10-16)

`logi_hidpp::onboard` : `Directory` et `Profile` gardent les octets bruts et ne réécrivent que les
champs modifiés (fréquence, DPI, paliers, boutons normaux / G-shift, 4 emplacements LED, nom,
animation), puis recalculent le CRC (`crc16` = CRC-16/CCITT-FALSE, valeur de contrôle 0x29B1).
Tests sur de vrais secteurs de la souris (`tests/data/`) : relu puis réécrit sans modification =
mêmes 255 octets. `mouse::Link::write_sector` : 0x8100 fn 6 (secteur, 0, longueur), fn 7 par blocs
de 16 octets, fn 8, puis relecture complète et comparaison ; les secteurs d'usine (0x0100+) sont
refusés.

Essai réel avec `hidpp-lab` (G HUB fermé ; l'outil refuse d'écrire tant que `lghub*.exe` tourne,
service de mise à jour compris) : profil d'essai écrit dans l'emplacement 3 (désactivé), activé,
souris turquoise fixe, puis `restore` : les 18 secteurs sont redevenus identiques à la copie de
l'étape 1. Constats :
- G HUB fermé, la souris repasse seule en « mémoire de la souris » sur le profil courant ;
- la souris **refuse de sélectionner un profil désactivé** (0x8100 fn 3 → InvalidArgument) : il
  faut l'activer dans l'annuaire (secteur 0) ;
- **octet 252 = animation personnalisée** (3 = dégradé rose → bleu « signature » d'usine) : tant
  qu'il n'est pas à 0, il recouvre les LED du profil (même règle que libratbag) ; les LED visibles
  sont l'emplacement 1 (+ 3 pour le mode économie) ;
- l'éclairage dynamique de Windows ne pilote pas cette souris ici (aucune entrée pour elle) ;
- `hidpp-lab` refuse un nouvel essai tant que le précédent n'a pas été annulé (un premier essai
  interrompu avait sinon pris le profil d'essai pour l'original ; il a été rétabli depuis la
  copie de l'étape 1 avec `restore-sector`).

### Étape 3 : la souris dans G13 Studio (0.8.0)

- **Thread `mouse`** (`mouse/worker.rs`, lancé par le `Runtime` si `RuntimeOptions::mouse`) : seul
  propriétaire de la souris, comme `g13-hid` pour le G13. Il la cherche toutes les 3 s (indices du
  récepteur sondés avec un délai court), lit à la connexion les réglages du **profil interne actif**,
  surveille le profil courant (bouton de la souris) toutes les 2 s, la batterie toutes les 60 s et
  G HUB toutes les 5 s, et publie un `MouseStatus` (`UiEvent::Mouse`, relayé par le moteur via
  `Request::Emit`). Trois requêtes sans réponse = souris partie (veille, hors de portée) : il la
  recherche ensuite.
- **Réglages** (`mouse/settings.rs`, `MouseSettings`) : 1 à 5 paliers DPI (valeurs alignées sur le
  pas du capteur), palier au démarrage, palier « sniper », fréquence 125–1000 Hz, éclairage
  (dégradé d'usine = animation 3, fixe, respiration, cycle, éteint ; un effet inconnu reste tel
  quel). `apply` ne touche que ces champs du profil : boutons et G-shift sont conservés (testé).
- **Écriture** (`MouseCommand::Apply`, bouton « Enregistrer dans la souris ») : refusée si G HUB
  tourne (`lghub.exe`, `lghub_agent.exe`, `lghub_system_tray.exe` ; le service de mise à jour
  ne compte pas), validation, copie complète de la mémoire si aucune n'existe encore, mode mémoire,
  écriture du secteur du profil actif + relecture, puis re-sélection du profil pour que la souris
  le recharge. Pas d'écriture au changement de jeu (usure de la mémoire flash).
- **G13 Studio** : onglets G13 / SOURIS dans l'en-tête (batterie à côté) ; page Souris : batterie,
  paliers (curseur + valeur, ◉ au démarrage, ◎ sniper, + / ×), fréquence, éclairage avec aperçu
  animé et la même fenêtre de couleur que le G13 (devenue générique : `openColor(target)`),
  « Annuler les modifications » / « Enregistrer dans la souris », bandeau si G HUB est ouvert.
  Commandes `get_mouse`, `apply_mouse`, événement `mouse`. Traduit dans les 5 langues.
- **LCD du G13** : l'applet Système affiche « SOURIS 93% » (« + » en charge) à la place de la mémoire.

### Étape 3 bis : changements en direct, comme G HUB (0.8.1)

Essais sur la souris (`hidpp-lab`, rien écrit en mémoire) :

| | mode mémoire (la souris suit son profil) | mode logiciel (« host », celui de G HUB) |
|---|---|---|
| DPI en direct (0x2201 fn 3) | accepté | accepté |
| Fréquence en direct (0x8060 fn 2) | **refusée** (InvalidArgument) | acceptée |
| Éclairage en direct (0x8071, contrôle logiciel pris) | accepté | accepté |
| Boutons DPI + / −, profil, G-shift | fonctionnent | **inactifs** (G HUB les intercepte) |
| Retour au mode mémoire | — | la souris reprend seule DPI, fréquence et éclairage de son profil |

Choix : rester en **mode mémoire** (tous les boutons de la souris marchent) et changer en direct ce
qu'elle accepte dans ce mode. `MouseCommand::Preview { lighting, dpi }` : éclairage via 0x8071
(fn 5 `01 03 04` pour prendre la main, fn 1 `zone, index de l'effet, 10 octets, 01` ; effets de la
G502 X PLUS : 0 éteint, 1 fixe, 0x0A respiration, 3 cycle ; `logi_hidpp::features::rgb`) et DPI
du palier en cours d'édition via 0x2201. `MouseCommand::Revert` (et la fermeture de G13 Studio) :
fn 5 `01 00 00` rend l'éclairage à la souris et la re-sélection du profil remet son DPI. Les aperçus
d'une rafale (curseur) sont fusionnés. La fréquence ne s'applique qu'à l'enregistrement (l'interface
le dit). Enregistrer écrit le profil puis rend la main : ce qui est affiché est ce qui est gardé.

**Effet d'usine** : copié octet pour octet du profil d'usine en ROM (secteur 0x0101 : 4
emplacements LED + octet d'animation 252) ; c'est aussi ce que contenaient les profils de
l'utilisateur (vérifié octet par octet contre la copie d'origine). Sur la souris, il donne des
couleurs unies qui changent avec le temps.

**Dégradé rose → bleu (0.8.2)** : celui que l'utilisateur voyait au début venait de **G HUB**, qui
colore les LED une par une en direct (PerKeyLightingV2 0x8081) ; il restait affiché un moment après
sa fermeture. G13 Studio fait pareil : `Lighting::Gradient { from, to, period_ms, reverse }` est
dessiné par le thread `mouse` à 10 images / s sur les 9 LED (ids 0 à 8 annoncés par 0x8081 fn 0) :
prise du contrôle 0x8071, effet firmware coupé (0x8071 fn 1 `FF, nb d'effets, 0…, 01`), puis 0x8081
fn 1 (4 LED par requête) et fn 7 (fin d'image) ; `settings::gradient_frame` calcule les couleurs
(de → vers → de le long de la bande, demi-boucle visible, glissement sans à-coup). Ce dégradé ne
peut pas être écrit dans la mémoire de la souris : il est gardé dans `<config>/mouse.toml` et
redessiné à chaque démarrage / reconnexion ; dans la souris, l'enregistrement met l'effet d'usine,
affiché quand G13 Studio ne tourne pas. Pas de dessin pendant que G HUB tourne. `hidpp-lab` utilise
le sw_id 0x0E (G13 Studio : 0x0D) pour pouvoir explorer pendant que G13 Studio tourne.

**Interface** : l'onglet porte le nom lu dans la souris (« G502 X PLUS ») et une icône de batterie
dessinée (niveau + éclair en charge) ; aperçus fidèles (dégradé rose → bleu qui glisse, cycle = une
seule couleur qui change de teinte) ; sensibilité en cartes « PALIER n » avec étiquettes PAR DÉFAUT /
SNIPER / EN COURS, boutons texte explicites et une phrase qui explique les boutons DPI de la souris.

### Étape 4 : remappage des boutons (0.9.0)

- **Dessin** (`ui/mouse-device.js`) : plan de la G502 X PLUS dans le style du G13, vue de dessus (G1,
  G2, molette G3 et ses inclinaisons, G9, G8 / G7 à gauche du clic gauche) et flanc gauche (G4, G5,
  G6) d'après les photos ; les petits boutons ont leur libellé à l'extérieur (traits de rappel
  pointillés, dessinés sous les boutons) ; la bande lumineuse reprend les couleurs choisies.
  Correspondance emplacement du profil → bouton, déduite des fonctions d'usine : 0 G1, 1 G2, 2 G3,
  3 G4 (précédent), 4 G6 (sniper), 5 G5 (suivant), 6 / 7 inclinaisons, 8 G9 (profil), 9 G8 (DPI +),
  10 G7 (DPI −). NORMAL / G-SHIFT : les deux tables de boutons du profil ; en G-SHIFT, le bouton qui
  porte la fonction G-shift est grisé.
- **Sélecteur d'action** : le même que pour le G13, devenu générique (`openPickerFor(target)`) ;
  pour la souris, onglets Clavier / Touches / Média / Souris seulement (sa mémoire ne garde ni macro
  à étapes ni bouton de manette) et, dans Souris, ses fonctions : DPI + / − / boucle / défaut,
  sniper, G-shift, profils + / − / boucle, batterie, inclinaisons de la molette.
- **Modèle** : `MouseSettings::buttons` / `gshift` = `Vec<Option<MouseAction>>` (mêmes formes JSON que
  les actions du G13 : `"LeftCtrl+C"`, `{ mouse }`, `{ special }`, `{ raw }` pour ce qu'on ne modifie
  pas, `null` = désactivé). Conversion vers la mémoire : `KeyCode::hid()` (nouveau : usage HID
  clavier, bit de modificateur ou usage « consumer » pour les touches multimédia ; testé pour
  toutes les touches) ; une seule touche par bouton (+ Ctrl / Maj / Alt / Win), sinon refus.
  Garde-fou : refus d'enregistrer un profil sans clic gauche. Le profil 2 réel de l'utilisateur
  est relu (Ctrl+T en G-shift…) puis réécrit à l'identique (test).
- Les boutons, comme les DPI, s'appliquent à l'enregistrement dans la souris (sa mémoire).

### Étape 5 : page souris refaite (0.10.0)

- **Mise en page** : celle du G13 — rail gauche (COUCHE N / G⇧, pastilles PROFILS), dessin au
  centre posé sur la grille (fond transparent), pied avec ANNULER / ENREGISTRER. Le dessin suit le
  plan fourni par l'utilisateur ; vue du dessus par défaut, bascule VUE DU DESSUS / PROFIL GAUCHE
  juste en dessous, puis le lien « Paramètres avancés ».
- **Paramètres avancés** (`#ms-adv`, `ui/mouse.js`) : onglets SENSIBILITÉ (barre DPI à échelle
  logarithmique : un clic pose un palier, 5 au plus, marqueurs déplaçables, valeur tapée, palier
  par défaut et sniper), ÉCLAIRAGE, FRÉQUENCE, PROFILS INTÉGRÉS (liste des 5 profils : afficher —
  les boutons affichés et modifiés sont ceux de ce profil —, utiliser, activer / désactiver) et
  MACROS (à venir).
- **Pastilles PROFILS** : verrouillées tant que les profils intégrés sont utilisés ; elles serviront
  aux profils G13 Studio en temps réel (un par application, le 1 partout par défaut), quand la
  souris sera pilotée en mode hôte.
- **Paramètres** : onglets GÉNÉRAL / CLAVIER G13 / SOURIS / CASQUE. SOURIS : export / import du
  profil affiché dans un fichier `.g13mouse` (JSON de `MouseSettings`, commandes
  `export_mouse_profile` / `import_mouse_profile`) ; l'import charge le brouillon, à enregistrer.

### Étape 6 : profils en temps réel (0.11.0)

- **Dessin** : tracé repris du plan SVG Gemini de l'utilisateur (vue du dessus et profil
  gauche, mêmes coordonnées) ; NORMAL / G-SHIFT en texte, en haut à droite de la page.
- **MouseButtonSpy (0x8110)**, sondé le 2026-10-17 : fn 0 = 11 boutons, fn 3 / fn 4 = table de
  réaffectation (bouton physique n → bouton HID n, 0 = rien), fn 1 / fn 2 = début / fin de
  l'espionnage ; événement 0 : masque 16 bits gros-boutiste, bit 0 = bouton 1 (clic gauche,
  mesuré). L'ordre des autres boutons est supposé être celui des emplacements du profil
  (`host::SPY_SLOTS`) ; le dessin allume le bouton pressé pour le vérifier.
- **Mode temps réel** (`mouse/host.rs`, `MouseCommand::SetHost`) : « Profils intégrés » décoché →
  la souris passe en mode hôte, G13 Studio applique son profil (fréquence, DPI, éclairage, table
  de réaffectation) et lit les boutons. Un bouton dont l'action est un simple bouton de souris
  reste câblé vers Windows (aucune latence, il marche même si G13 Studio s'arrête) ; les autres
  sont muets et leur action est faite par le fil souris : touches (injecteur partagé avec le
  moteur), paliers DPI, sniper, G-shift, profil suivant / précédent, défilement horizontal.
  Tout ce qu'un bouton tient est relâché au changement de profil, à la perte de la souris et à
  l'arrêt ; l'arrêt rend la souris à sa mémoire (table identité, mode mémoire, profil rechargé).
  En temps réel, un bouton sans action G-shift garde son action normale.
- **Profils G13 Studio** : `<config>/mouse-profiles.json` (`host`, `profiles` : nom, applications,
  `MouseSettings`), 5 au plus ; le profil 1 sert partout, un autre s'active quand l'une de ses
  applications est au premier plan (exécutable fourni par le moteur, `Focus`). Pastilles PROFILS
  (ajout = copie du profil affiché), onglet APPLICATIONS des paramètres avancés (applications
  détectées, parcourir, suppression du profil). Commandes `set_mouse_host`, `save_mouse_soft`,
  `delete_mouse_soft`.
- `Link` garde les notifications reçues pendant l'attente d'une réponse (`Link::event`) : aucun
  appui n'est perdu pendant une lecture de batterie.

Vérifié sur la souris le 2026-10-17 : l'ordre des boutons (`SPY_SLOTS`) est le bon. En temps réel, un
clic sur la page revient aussi comme « bouton pressé » : ces mises à jour n'allument que le dessin
(sans redessiner la page, sinon le clic serait perdu). Sélecteur d'appareil : miniatures du G13
(contour du dessin) et de la G502 (vue du dessus) ; sur la page souris, le bandeau du haut est celui du G13 adapté (`.ms-pbar`) : numéro et nom du profil,
nouveau (boutons d'usine), dupliquer, renommer, importer / exporter (`.g13mouse`), effacer les boutons
(clic gauche, clic droit et molette gardés pour que la souris reste utilisable), supprimer (pas le
profil 1). Les actions de profil G13 Studio ne sont actives qu'en temps réel.

### Étape 7 : macros et RECORD (0.12.0)

- **Enregistrement** : `platform::start_recording` (Windows : hooks `WH_KEYBOARD_LL` / `WH_MOUSE_LL` sur un
  fil à boucle de messages, `platform/windows/recorder.rs`) capture les touches (code de balayage →
  `KeyCode`), les boutons de souris et la molette, pas le déplacement ni ce que G13 Studio injecte
  (indicateurs « injected »). Linux : lecture evdev de `/dev/input/event*` sans les accaparer
  (`platform/linux/recorder.rs`), périphériques virtuels de G Nexus et G13 exclus, délais tirés des
  horodatages du noyau ; accès par la règle optionnelle `deploy/linux/71-gnexus-record.rules`.
  `recording::to_steps` en fait des étapes (`down A`, `wait 82`…) : délais
  réels ou fixes (25 ms), répétition automatique ignorée, relâchements orphelins retirés, clic sur
  STOP retiré, tout ce qui reste appuyé relâché à la fin. Commandes `record_start` / `record_stop`.
- **Éditeur commun** (`ui/macro-editor.js`, classe `MacroEditor`) : celui du G13, rendu générique, utilisé
  par le G13 et par la souris ; bouton RECORD / STOP (remplace les étapes), « garder les délais réels ».
- **Macros de la souris** (temps réel seulement) : `MouseSettings::macros` (même forme que le G13),
  action `{ "macro": nom }` ; refusée pour la mémoire de la souris. Le fil souris les joue avec le
  lecteur du moteur (`engine::macros::spawn`) et les mêmes modes (une fois, répéter tant que maintenu,
  marche / arrêt) ; changement de profil, perte de la souris et arrêt les annulent (touches relâchées).

### Étape 8 : bibliothèque de macros, profil par défaut, overlay souris (0.13.0)

- **Bibliothèque de macros** (`<config>/macros.json`, `g13-gui/src/library.rs`) : toutes les macros
  faites dans G13 Studio, G13 et souris, remplie au départ avec celles des profils existants. Chaque
  profil garde sa copie (le moteur et le fil souris lisent le profil) ; la bibliothèque garde la
  dernière version de chaque nom (`MacroLib` dans `macro-editor.js` : mise à jour à chaque
  modification, renommage compris). Éditeur et sélecteur listent sous BIBLIOTHÈQUE les macros des
  autres profils : un clic en met une copie dans le profil. Commandes `macro_library`,
  `save_library_macro`, `delete_library_macro`.
- **Profil par défaut et « Tester partout » pour la souris** : `host::choose` (profil testé partout,
  sinon celui de l'application, sinon le profil par défaut, `HostFile::default`) ;
  `MouseCommand::SetSoftDefault` / `PinSoft` (le test n'est pas enregistré). Barre de la souris :
  TESTER PARTOUT / UTILISER PAR DÉFAUT / ASSOCIER À UN JEU et la phrase qui dit où le profil
  s'applique, comme pour le G13. Le profil par défaut ne peut pas être supprimé.
- La fenêtre de G13 Studio (et l'overlay) ne change plus le profil de la souris : elle garde celui
  du jeu (`ProfileManager::is_self`).
- **Overlay** : les onglets G13 / SOURIS de la barre du haut sont repris dans son en-tête ; sur la
  souris, il montre ses pastilles de profils, le dessin, NORMAL / G-SHIFT et « Associer à ce jeu ».

## Casque PRO X Wireless (HID++ 2.0) — étape 1 (0.14.0)

- **Sondage** (2026-10-17, `hidpp-probe`, lecture seule) : le récepteur USB 046D:0ABA répond en HID++ 4.2 à
  l'index 0xFF sur la collection 0xFF43 / 0x0202 (interface 3), rapport long 0x11. Fonctions :
  0x1F20 mesure de tension (fn 0 : mV gros-boutiste + état, 3 = en charge ; fn 1 / fn 2 : délai de
  mise en veille en minutes, 0 = jamais), 0x8300 retour micro (fn 0 / fn 1 : 0..100), 0x8310 égaliseur
  intégré (fn 0 : 10 bandes, ±12 dB ; fn 1 : fréquences 7 par appel, 32 Hz … 16 kHz ; fn 2 : gains ;
  fn 3 : `02` + un octet signé par bande). Casque éteint : le récepteur répond par une erreur HID++.
  Sources : HeadsetControl (`logitech_gpro.hpp`, table batterie) et Solaar (`settings_templates.py`).
- Les trois écritures ont été vérifiées sur le casque (relues, puis remises à leur valeur).
- **Module `headset`** (`g13-daemon/src/headset/`) : `find_headsets`, `Headset` (lecture / écriture,
  relue à chaque fois), `battery_percent` (courbe de HeadsetControl), fil `HeadsetWorker` (seul
  propriétaire du récepteur : casque allumé / éteint toutes les 3 s, batterie toutes les 30 s),
  `UiEvent::Headset`, `Runtime::headset`. Outil `headset-lab` (état, `sidetone`, `sleep`, `eq`).
- **G13 Studio** : onglet CASQUE (miniature) dans la barre du haut et l'overlay (`showDevice`), page
  Casque (`ui/headset.js`) : dessin de face, batterie, retour micro, mise en veille, égaliseur 10 bandes
  avec préréglages ; chaque réglage est envoyé tout de suite (comme G HUB). Commandes `get_headset`,
  `set_headset_sidetone`, `set_headset_sleep`, `set_headset_eq`.
- **Volume** (0.14.1) : volume Windows de la sortie du casque (Core Audio, `platform::output_volume` /
  `set_output_volume`, sortie trouvée par son nom, « Casque (2- Logitech PRO X Wireless Gaming Headset) ») ;
  relu toutes les 0,7 s, il suit la molette du casque. Réglage et coupure depuis la page Casque.
  Linux : PipeWire par `pactl -f json` (`platform/linux/audio.rs`) ; la sortie est trouvée par ses
  propriétés (`device.description`, `node.nick`), car `pactl` met à `null` les textes non ASCII.
  Le micro (retour) est replié en bas de la page : l'utilisateur n'utilise pas le micro du casque.
- **Égaliseur intégré du casque (0x8310)** : il garde les réglages mais ne change pas le son (tests
  d'écoute du 2026-10-17 ; seul le préfixe `02` est accepté). G HUB ne s'en sert pas non plus.
- **Égaliseur du système** (0.14.2) : la sortie du casque porte l'effet audio de Logitech « HX2E Surround
  Sound Effect » (APO installé avec le pilote de G HUB). Son égaliseur se règle dans la valeur
  `{42d67d07-c2b8-47f7-8783-109668632ec6},0` de `MMDevicesAudioRender{sortie}FxProperties` (HKLM, que les
  utilisateurs ont le droit de modifier) : `u32` 65, `u32` 1, 10 fréquences (Hz), 10 gains (dB), 10 Q, en
  `f32`, puis des champs laissés tels quels. L'effet relit la valeur en direct (test au souffle joué
  directement sur la sortie : gains à -12 dB entendus aussitôt), sans droits administrateur ni G HUB.
  `platform::output_eq` / `set_output_eq` ; le fil casque s'en sert pour la page Casque.
  Limite : un logiciel qui envoie son son au casque sans passer par le mixeur de Windows (mode exclusif
  ou brut, comme Wave Link) contourne les effets, donc l'égaliseur. Linux : PipeWire (filter-chain), plus tard.
- Hors d'atteinte sans pilote : son surround DTS et filtres Blue VO!CE (traitement logiciel de G HUB).

### Égaliseur de G13 Studio (0.15.0)

- **Constat** (tests d'écoute du 2026-10-17) : l'égaliseur intégré du casque ne change pas le son ;
  G HUB pilote son effet audio Logitech (HX2E) par un canal privé (aucune commande vers le casque, réglage
  Windows de l'effet inchangé pendant que l'égaliseur de G HUB agit). Écrire ce réglage ne s'entend pas.
- **`g13-dsp`** (sans code système) : égaliseur graphique à filtres en cloche (RBJ), 10 bandes de 32 Hz à
  16 kHz, ±12 dB, marge automatique (le son baisse du plus grand gain positif : pas de saturation),
  `process` sans allocation ; réglages `EqSettings` (fichier binaire `G13EQ1`). Testé.
- **`g13-apo`** (DLL, Windows) : effet audio (APO) chargé par le moteur audio de Windows sur la sortie du
  casque, en *effet de point de terminaison* (EFX, `{d04e05a6-…},7`) : l'effet Logitech (SFX) reste en
  place. Il relit `%ProgramData%G13 Studioaudio-eq.bin` dès qu'il change (fil de surveillance, 150 ms) ;
  le fil audio prend les réglages sans attendre (`try_lock`) et n'alloue rien. Float 32 bits seulement,
  en place. Testé par un hôte qui joue le rôle du moteur (`tests/host.rs` : coupe de -12 dB à 4 kHz, retour
  à plat en direct).
- **Installation** (`g13_apo::setup`, droits administrateur, une fois) : DLL copiée dans
  `%ProgramFiles%G13 Studio`, dossier de réglages ouvert en écriture aux utilisateurs, enregistrement COM
  et APO, emplacement EFX de la sortie (l'ancienne valeur est gardée pour la désinstallation), redémarrage
  du service audio. Les réglages de protection audio de Windows ne sont pas touchés. G13 Studio embarque
  la DLL (construite par `g13-gui/build.rs`) et se relance lui-même en administrateur
  (`--audio-setup install|uninstall`, `platform::windows::elevate::run_elevated`).
- **Page Casque** : bouton INSTALLER L'ÉGALISEUR tant que l'effet n'est pas installé, puis curseurs et
  préréglages (fichier écrit par le fil casque, `headset::eq`), DÉSINSTALLER L'ÉGALISEUR.
- **Vérifié sur la machine (0.15.4, 2026-10-17)** : le moteur audio charge l'effet non signé sans toucher à
  la protection audio de Windows, et l'égaliseur s'entend en direct. Deux pièges rencontrés :
  - l'emplacement EFX doit être accompagné de ses modes de traitement
    (`{d3993a3f-99c2-4402-b5ec-a92a0367664b},7` = `AUDIO_SIGNALPROCESSINGMODE_DEFAULT`, REG_MULTI_SZ), sinon
    l'effet est ignoré ;
  - le moteur audio crée ses effets par **agrégation COM** (objet extérieur, `IUnknown` demandé d'abord) :
    les objets `windows-rs` ne le permettent pas, la couche COM de `g13-apo` est donc écrite à la main
    (`IUnknown` non délégant, interfaces déléguant à l'objet extérieur ; test `aggregated_like_the_audio_engine_does`).
  - Journal de l'effet : `%ProgramData%G13 Studioapo.log` (chargement, format, réglages lus). Lien
    RÉINSTALLER sur la page pour mettre l'effet à jour en une seule autorisation.
- Linux : `g13-daemon` ne dépend de `g13-apo` que sous Windows. Les réglages sont rangés dans
  `~/.config/g13d/audio-eq.bin` (même format) et pilotent un filtre PipeWire (`platform/linux/eq.rs`) :
  fichier `~/.config/pipewire/filter-chain.conf.d/gnexus-eq.conf` lu par le service utilisateur
  `filter-chain.service` (activé à l'installation), *smart filter* WirePlumber placé devant la sortie du
  casque (`filter.smart.target = { node.name = … }`, la sortie par défaut ne change pas), graphe
  `bq_highshelf` à 0 Hz (préampli = `preamp_db`) puis 10 `bq_peaking` (fréquences et Q de `g13-dsp`).
  Gains appliqués en direct (`pw-cli set-param gnexus_eq Props`) et réécrits dans le fichier. Comme
  l'effet Windows, il reste actif G Nexus fermé. Installation / désinstallation sans droits
  (`headset::eq::install` / `uninstall`, commandes `audio_eq_install` / `audio_eq_uninstall`) ; la
  désinstallation redémarre le service sans le désactiver (`Also=pipewire.socket`).

### Profils d'égaliseur (0.16.0)

- **Molette** : elle envoie des touches multimédia Volume + / − (collection 0x000C du récepteur, rapport
  `01 01` / `01 02` / `01 00`) ; abandonnée : la molette de l'utilisateur est défaillante. Le bouton du
  micro ne passe pas par le PC. `headset-lab listen` et `wheel-test` restent pour l'étude.
- **Profils** (`headset::eq::EqProfiles`, `<config>/headset-eq.json`, 8 au plus) : nom, applications,
  gains. Le profil par défaut ★ s'applique partout, un profil associé à une application quand elle est
  au premier plan (même `Focus` que la souris : la fenêtre de G13 Studio ne compte pas), « Tester partout »
  (non enregistré) passe avant. Le fil casque écrit les gains du profil choisi pour l'effet audio.
  Commandes `save_eq_profile` (le profil enregistré s'entend aussitôt), `delete_eq_profile`,
  `use_eq_profile`, `set_eq_default`, `pin_eq_profile`.
- **Page Casque** : pastilles à gauche, bandeau du haut (nouveau, dupliquer, renommer, supprimer — pas le
  profil par défaut), phrase « où s'applique ce profil » avec TESTER PARTOUT / UTILISER PAR DÉFAUT /
  ASSOCIER À UN JEU, section APPLICATIONS DE CE PROFIL ; le profil en cours est affiché sur le logo G de
  l'écouteur droit du dessin.

- **0.16.1** : le profil de l'appareil affiché (G13, souris, casque) et ses boutons sont sur un bandeau à part
  sous l'en-tête (`.profile-row`), qui ne garde que le logo, les onglets d'appareils, l'état et les paramètres :
  plus de chevauchement dans une fenêtre étroite. Dessin du casque repris du plan SVG Gemini de l'utilisateur
  (vue 3/4 par défaut, profil gauche avec les commandes) ; le disque G de l'écouteur droit porte le profil
  d'égaliseur en cours.

- **0.17.0** :
  - casque : vue 3/4 seule, centrée comme le G13 et la souris, réglages en dessous ; coque de l'écouteur
    droit redessinée au plus près du coussin ; textes « Arceau » et « Micro amovible » retirés ;
  - import / export des profils d'égaliseur (fichier `.gneq`, JSON `{format: "gnexus-eq", version: 1,
    profile}`, gains bornés à ±12 dB, 10 bandes) depuis le bandeau du casque et les Paramètres ;
    commandes `export_eq_profile` / `import_eq_profile` ; un profil importé est ajouté, sans application ;
  - touches multi-états du G13 (voir « Profils ») ;
  - le logiciel s'appelle **G Nexus** (nom du produit, de l'exécutable, fenêtre, textes). Les chemins internes
    ne changent pas : identifiant `org.g13.studio` (donc même dossier de configuration),
    `%ProgramData%\G13 Studio` et `%ProgramFiles%\G13 Studio\g13_apo.dll` de l'égaliseur,
    `HKLM\SOFTWARE\G13 Studio\AudioEffect`. L'installateur pose `G Nexus` à côté de l'ancien
    « G13 Studio », à désinstaller à la main.

- **0.17.1** : logo en haut à gauche et icônes de l'application (fichiers générés par
  `icons/make_icon.ps1`) : le même hexagone, en bleu #1E56E4 ; casque : le coussinet droit suit le trait
  blanc de la silhouette.

- **0.18.0** :
  - mémoire : fermer la fenêtre détruit sa web view (`"create": false` dans tauri.conf.json, `show_main`
    la recrée depuis la configuration ; `RunEvent::ExitRequested` sans code est empêché quand « fermer réduit
    dans la zone de notification » est actif). L'overlay n'est créé qu'à l'appui sur sa touche et détruit à
    sa fermeture : sa page appelle `overlay_ready` une fois chargée, qui rend ce qu'elle doit afficher
    (`AppState::overlay_pending`) puis place et montre la fenêtre. Mesure : 217 Mo privés fenêtre ouverte,
    **7 Mo** (45 Mo working set, un seul processus) fenêtre fermée ;
  - sauvegardes automatiques (`backup::auto_backup`, fil `spawn_auto_backup` : une minute après le démarrage
    puis chaque heure) dans `<config>/backups/auto/` : chaque jour / semaine / mois (`Settings::auto_backup`,
    défaut semaine), seulement si la configuration a changé ; les `auto_backup_keep` plus récentes sont
    gardées (défaut 10) ; « Sauvegarder maintenant » (`backup_now`). Une sauvegarde contient aussi
    `mouse-profiles.json`, `headset-eq.json` et `macros.json` (`Backup::files`, restaurés à l'import et pris
    en compte au prochain démarrage).

**Suite** : Linux (PipeWire pour l'égaliseur).


## Import / export Logitech Gaming Software

`g13-daemon/src/profiles/lgs.rs` (pur Rust, `roxmltree`), boutons ⤓ / ⤒ de l'en-tête. Format LGS 9.x
(espace de noms `…/Cassandra/2010.7/Profile`), relevé sur un vrai profil exporté :

| LGS | G13 Studio |
|---|---|
| `<assignments devicecategory="Logitech.Gaming.LeftHandedController">` | le G13 (les autres catégories sont ignorées) |
| `shiftstate` 1 / 2 / 3 | menus M1 / M2 / M3 |
| `contextid` G1–G25 · G26 / G27 / G28 / G29 | G1–G25 (G25 = clic stick) · stick haut / droite / bas / gauche |
| affectation `backup="false"` (personnalisée avant d'origine) | active ; `backup="true"` = ancienne valeur gardée par LGS, ignorée |
| `<keystroke>` (une ou plusieurs `<key value>`) | touche / combinaison — noms LGS = positions physiques US, comme `KeyCode` |
| `<mousefunction>` leftclick / rightclick / middleclick / back / forward | boutons souris |
| `<hotkeys>` copy / paste / cut | Ctrl+C / V / X |
| `<function>` média | touches média |
| `<backlight devicemodel="…G13"><mode color shiftstate>` | couleur de chaque menu |
| `<target path>` (sauf `LCORE.EXE`) | application associée (nom de l'exe) |

Non converti, et listé dans un rapport plutôt que deviné : macros à séquence (`multikey`), blocs de
texte, scripts Lua, raccourcis dépendant de la disposition (`undo`, …) ; à l'export : molette,
macros à étapes et boutons de manette. Les noms LGS des touches rares (pavé numérique, touches spéciales) suivent la
nomenclature de l'API Lua de LGS : l'import accepte plusieurs variantes.

## Profils

Un fichier `<id>.toml` par profil, dans `%APPDATA%\g13d\config\profiles\` (Windows) ou
`~/.config/g13d/profiles/` (Linux) ; `--profiles <dossier>` en choisit un autre. Si le dossier est
vide, `demo.toml` est généré depuis `engine/demo.rs`. Si aucun fichier n'est lisible, le profil de
démo est utilisé en mémoire.

```toml
name = "Jeu"
default = true                  # profil utilisé quand aucune règle ne correspond

[[rules]]                       # bascule auto quand une de ces applis a le focus
exe = "eldenring.exe"           # nom de fichier, casse ignorée, ".exe" facultatif
[[rules]]
title = "Mozilla Firefox"       # sous-chaîne du titre de fenêtre, casse ignorée

[banks.M1]                      # M1 / M2 / M3
color = "#FF8000"
G1 = "Escape"                   # touche ou accord : "Ctrl+Shift+Esc" (positions US : W = Z en AZERTY)
G2 = { mouse = "Right" }        # Left / Right / Middle / Back / Forward
G3 = { wheel = -1 }             # crans par appui ; horizontal = true pour l'inclinaison
G4 = { macro = "spam" }         # défini dans [macros]
G5 = { pad = "A" }              # bouton de manette virtuelle (voir « Mode manette »)
G6 = { tap = "D", hold = "LeftCtrl", double_tap = { macro = "spam" }, tap_hold = "Escape" }

[macros.spam]
mode = "repeat_while_held"      # once / repeat_while_held / toggle
steps = ["down Q", "wait 20", "up Q", "wait 80"]
# étapes : down/up <touche>, mouse_down/mouse_up <bouton>, wheel <n>, wait <ms>,
#          wait_random <ms> <marge> (pause de <ms> plus 0 à <marge> ms tirés à chaque passage)
```

**Touche multi-états (tap dance)** : `{ tap, hold, double_tap, tap_hold }`, chaque champ facultatif et
contenant une action ordinaire (pas un autre tap dance). Le moteur (`Action::TapDance`, `engine::Dance`
par emplacement, `TAP_TERM` = 200 ms, vérifié à chaque tick de 10 ms) distingue :
- appui court relâché, sans second appui dans les 200 ms → **tap** (appui + relâchement) ;
- appui tenu 200 ms → **hold**, tenu jusqu'au relâchement ;
- deux appuis courts → **double_tap** ; second appui tenu 200 ms → **tap_hold** (tenu) ;
- un hold ou un tap-hold vide retombe sur le tap ; sans aucun autre mode qu'un tap, l'action est ordinaire (réaction immédiate,
  aucun délai). Une macro dans un tap dance est jouée une fois. Changement de profil, déconnexion et arrêt
  remettent tout à zéro (rien ne reste enfoncé). L'export LGS ne garde que le tap.

Dans l'interface, le sélecteur d'action d'une touche du G13 a à gauche une grille 2×2 Tap / Hold / Tap-Tap /
Tap-Hold : on choisit un mode, puis son action ; le × vide un mode. Une touche multi-états affiche son tap
suivi de « ⁺ ».

Touches identifiées par `G13Key::id()` (`G1`…`G25`) ; M-keys, MR, BD et L1–L4 ne sont pas
assignables. Les erreurs (touche ou accord inconnu, macro non définie, champ mal orthographié…)
sont signalées avec leur emplacement ; un fichier invalide ne remplace jamais sa dernière version
valide.

Sélection : `ProfileManager::tick` est appelé par `Engine::run_with` à chaque tick (≤ 10 ms) sur le
thread moteur. Toutes les 250 ms, il lit l'application au premier plan (`ForegroundWatcher`) ;
le premier profil (ordre alphabétique des ids) dont une règle correspond est activé, sinon le
profil par défaut (`default = true`, sinon `demo`, sinon le premier). Le profil par défaut, lui,
peut aussi avoir des règles. Chaque seconde, le dossier est relu (date + taille) : un profil ajouté,
modifié ou supprimé est pris en compte sans redémarrage. Si le profil actif a changé, il est
réappliqué ; s'il a disparu, on revient au profil par défaut. Toute bascule passe par
`Engine::set_profile`, qui relâche les touches et arrête les macros (la banque M est conservée).
`--no-auto` désactive la bascule automatique.

Validé sur le G13 (2026-10-16) : passage vert/orange en entrant/sortant du Bloc-notes, G1 → F5
propre au profil, ajout d'un fichier et changement de couleur pris en compte à chaud.

## Validation Windows (phase 1, 2026-10-15)

Windows 11, rustc 1.99 (toolchain GNU). `cargo build`, `clippy --all-targets` et `test` passent
sur tout le workspace ; `platform/windows/*` a compilé sans correction. Testé sur un G13 réel :

| Point | Résultat |
|---|---|
| Fronts appui/relâchement (G1–G25, M1–MR, BD, L1–L4) | OK après correction de la carte des bits |
| LEDs M1–M3/MR (0x05), couleur par banque (0x07), BD | OK |
| Mode interruption | OK **si Logitech Gaming Software / G HUB sont fermés** |
| `--poll` | OK (même avec LGS lancé) |
| Bloc-notes, AZERTY : M1+G10 → `z`, M2+G1/G2 copier/coller, M3+G2 autoclic | OK |
| Arrêt Ctrl+C | OK, tout est relâché |
| Arrêt par SIGTERM / SIGHUP (Linux) | `g13d` et l'appli passent par l'arrêt propre (`ctrlc`, feature `termination` sous Unix seulement) : code de sortie 0 au lieu de 143 |
| Débranchement / rebranchement à chaud | OK (2026-10-16) : G1 tenue (Ctrl+C) relâchée au débranchement, attente sans erreur, banque M2 + couleur réappliquées au rebranchement |

Constats matériels :
- La spec d'origine décalait G23–G25, BD, L1–L4, M1–M3 et MR ; les bits d'état (octet 5 bit 7,
  octet 7 bits 5–7) faisaient voir G24 enfoncé en permanence. Spec corrigée d'après captures.
- **LGS / G HUB** captent les rapports d'interruption : `g13d` ne reçoit alors rien (sauf `--poll`).
  Ils doivent être fermés (et à terme désinstallés) pour que `g13d` les remplace.
- Blanc `(255,255,255)` rendu rosé par les LEDs ; une correction d'équilibre sera utile.
- Rétroéclairage à `(0,0,0)` (BD) : les LEDs M s'éteignent aussi.
- La toolchain GNU échoue sur les chemins longs (MAX_PATH) : garder le projet sur un chemin court.

## Étapes suivantes

1. ~~**Profils**~~ : fait (voir « Profils »).
2. **Stick** : mode touches (`STICK_UP/DOWN/LEFT/RIGHT`) et mode manette Xbox 360 (ViGEmBus,
   calibration automatique) faits. Reste : manette uinput sous Linux ; éventuellement courbe de
   réponse / sensibilité et stick → souris.
3. **MR** : enregistrement à la volée via hook `WH_KEYBOARD_LL` (filtrer `dwExtraInfo == "G13D"`)
   / lecture evdev sous Linux.
4. ~~**LCD**~~ : fait (voir « Écran LCD »). Pistes : applets configurables, contrôle média par
   boutons, métadonnées MPRIS sous Linux.
5. **IPC** (seulement si l'on veut une UI séparée de `g13d`, par ex. service système Linux) :
   JSON-RPC sur Named Pipe / socket Unix ; G13 Studio embarque aujourd'hui le runtime directement.
6. **Portage Linux** : remplir `platform/linux/*`, service systemd utilisateur.
