# Portage Linux (CachyOS, Bazzite)

Objectif : G Nexus fait sous Linux tout ce qu'il fait sous Windows, pour les trois appareils, avec la même
interface. Cible : CachyOS (Arch), session **Wayland** (KDE Plasma ou GNOME), **PipeWire**.
La version Windows reste la référence : chaque étape doit laisser `cargo build` Windows intact.

## État de départ

- Moteur, profils, macros, tap-dance, LCD, HID++ (souris, casque) : code commun, déjà portable (`hidapi`).
- `crates/g13-daemon/src/platform/linux/` : **ébauches** (`injector.rs`, `foreground.rs`, `media.rs`).
- `deploy/linux/` : règles udev (G13) et `uinput.conf`, à compléter pour la souris et le casque.
- Code Windows seulement : `platform/windows/*` (injection SendInput, manette ViGEm, enregistreur de macros,
  premier plan, média, volume, élévation), `g13-apo` (égaliseur = effet audio Windows), parties
  `#[cfg(windows)]` de `g13-gui/src/main.rs` et de `headset/eq.rs`.

## Étapes proposées (dans l'ordre)

1. **Compiler** : `cargo build --workspace` sous Linux. Dépendances système (paquets Arch) :
   `webkit2gtk-4.1 gtk3 libayatana-appindicator hidapi systemd-libs base-devel`. Exclure `g13-apo`
   du build Linux (crate Windows seulement) sans casser Windows.
   **Fait (2026-10-17).**
   - `.cargo/config.toml` fixe un `target-dir` Windows (`C:/…`), inutilisable sous Linux (le `:` casse
     `LD_LIBRARY_PATH`). Il est gardé tel quel pour Windows ; sous Linux, surcharger :
     `set -Ux CARGO_TARGET_DIR ~/gnexus/target` (fish) ou `export CARGO_TARGET_DIR=…` (bash/zsh).
   - `g13-apo` n'est plus une dépendance Linux de `g13-daemon` (`[target.'cfg(windows)']`). Il reste
     membre du workspace (le `build.rs` Windows de `g13-gui` le compile avec `-p g13-apo`) : sous Linux,
     il ne compile que des fonctions portables, sans effet. Réglages de l'égaliseur sous Linux :
     `~/.config/g13d/audio-eq.bin`.
   - Exécution : `libayatana-appindicator` est indispensable (sans lui, Tauri s'arrête en créant l'icône
     de notification).
   - Vérifier Windows depuis Linux : `rustup target add x86_64-pc-windows-gnu`, puis
     `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu`. Sans `mingw-w64-gcc`, de
     faux `x86_64-w64-mingw32-gcc` et `-windres`, qui écrivent des sorties vides, suffisent pour un
     *check* (aucune édition de liens), plus un `g13_apo.dll` vide dans `out/apo-target/release` de `g13-gui`.
2. **Accès aux appareils** : règles udev `uaccess` (hidraw) pour le G13 (046D:C21C), la G502 X PLUS
   (filaire et récepteur LIGHTSPEED) et le dongle du casque (046D:0ABA) ; `/dev/uinput` accessible.
   Vérifier avec les binaires de diagnostic (`hidpp_lab`, `headset_lab`).
   **Fait (2026-10-17)**, sur une VM VMware avec les trois appareils transmis (`usb.generic.allowHID`).
   - `deploy/linux/70-gnexus-uaccess.rules` (recommandé, ACL de session) ou
     `99-gnexus-input-group.rules` (groupe `input`) ; installation dans `deploy/linux/README.md`.
     `/dev/uinput` est accessible à l'utilisateur.
   - G13 : `hid-lg-g15` se lie au G13 mais ne gêne pas : hidraw reste lisible et écrivable
     (`g13d --dry-run -vv` : touches G, stick, LED M, rétroéclairage, LCD).
   - Souris : `hid-logitech-dj` crée un appareil `046D:4099` (G502 X PLUS) sous le récepteur `C547` ;
     la règle du récepteur le couvre (`ATTRS` remonte au parent USB). HID++ répond à l'index 1, par
     ce nœud comme par celui du récepteur. Vu : batterie, DPI, fréquence, mémoire interne.
   - Casque : toutes les features répondent casque allumé (éteint : `LogitechInternal`, normal).
   - Sous Linux, `hidapi` donne une entrée par collection HID d'un même `/dev/hidraw` : `find_mice` et
     `find_headsets` filtrent désormais l'usage (0x0002 / 0x0202) partout, plus seulement sous Windows.
     Sans ce filtre, la souris était sondée 5 fois (et le G13 7 fois en vain) à chaque recherche : elle
     est trouvée en 0,25 s au lieu de 4 s.
   - Arrêt par SIGTERM / SIGHUP (fermeture de session, `kill`) : `g13d` et l'appli s'arrêtent proprement
     (`ctrlc` avec `termination`, sous Unix seulement).
3. **Injection** (`platform/linux/injector.rs`) : clavier + souris via **uinput** (crate `evdev`),
   codes evdev = positions physiques. Relâchement garanti à l'arrêt.
   **Fait (2026-10-17).** Deux périphériques uinput (« G Nexus virtual keyboard » et « … mouse »,
   `BUS_VIRTUAL`) ; la molette envoie `REL_WHEEL` et `REL_WHEEL_HI_RES` (120 par cran), comme une vraie.
   Test : `cargo run -p g13-daemon --example uinput_probe` dans une fenêtre qui a le focus.
   - AZERTY vérifié : `W Q A Z 1` donnent `z a q w &`, Maj+W `Z` ; boutons 1/3/2/8/9, déplacements,
     molettes verticale et horizontale (vérifiée sous XWayland ; sous Wayland, GTK3 affiche à l'envers
     la direction *discrète* horizontale, mais la valeur continue est juste).
   - `uinput_probe hold` : W tenue puis SIGKILL → le noyau envoie le relâchement, plus de répétition.
   - `g13d` et l'appli créent les deux périphériques au démarrage et les retirent à l'arrêt.
4. **Manette virtuelle** (mode stick) : périphérique uinput type Xbox 360 (remplace ViGEm).
   **Fait (2026-10-17).** `platform/linux/gamepad.rs` : même identité, mêmes axes et boutons que le
   pilote `xpad` (`045E:028E`, sticks ±32768 avec Y vers le bas, gâchettes 0..255, croix sur le chapeau),
   branchée à la première mise à jour, retirée par `gamepad_unplug`. Test :
   `cargo run -p g13-daemon --example uinput_probe -- pad` (relit la manette dans `/dev/input`,
   accessible grâce au `uaccess` que systemd donne aux manettes).
   Reste pour l'étape 9 : les textes de l'interface parlent encore de ViGEmBus.
5. **Enregistreur de macros (RECORD)** : lecture evdev du clavier / de la souris (groupe `input` ou uaccess).
   **Fait (2026-10-17).** `platform/linux/recorder.rs` lit tous les claviers et souris sans les
   accaparer (non bloquant, toutes les 5 ms), en excluant les périphériques virtuels de G Nexus et le G13
   (que `hid-lg-g15` expose aussi) ; les délais viennent des horodatages du noyau, la répétition
   automatique et les déplacements sont ignorés.
   - Accès (choix de l'utilisateur : uaccess) : `deploy/linux/71-gnexus-record.rules`, optionnelle.
     Sans elle, RECORD répond « aucun accès à /dev/input : installez … ».
   - Test : `cargo run -p g13-daemon --example record_probe` (un clavier de test tape pendant que
     l'injecteur de G Nexus tape E) → `down W / wait 50 / up W / wait 300 / down Q / up Q / wait 40 /
     mouse_down Right / mouse_up Right` : délais exacts, E de G Nexus exclu.
6. **Application au premier plan** (profils par jeu) : sous Wayland il n'y a pas d'API générique.
   Pistes : KWin (script D-Bus) pour Plasma, extension / `org.gnome.Shell` pour GNOME, `/proc` pour le
   nom de l'exécutable ; jeux Proton/Wine = nom de l'`.exe` Windows (utile pour garder les mêmes règles).
   Prévoir un repli propre si rien n'est disponible.
   **Fait pour KDE Plasma (2026-10-17)**, Plasma 6 (noms Plasma 5 en repli dans le script).
   - `platform/linux/foreground.rs` : script KWin (`$XDG_RUNTIME_DIR/gnexus-focus.js`, extension
     `gnexus-focus`) qui appelle G Nexus par D-Bus (`zbus`) à chaque changement ; pas de sondage.
     Retiré à l'arrêt ; un script laissé par un plantage est remplacé au démarrage suivant.
   - Nom de l'application : le `.exe` Windows trouvé dans `/proc/<pid>/cmdline` (Wine / Proton), sinon
     `/proc/<pid>/exe` ; `app_id` = classe de la fenêtre (Wayland app-id / WM_CLASS).
   - `running_apps` (onglet Applications) : fenêtres de la barre des tâches ayant un titre.
   - Test : `cargo run -p g13-daemon --example foreground_probe -- 20`. Vérifié avec `g13d` : une
     fenêtre dont la ligne de commande contient `C:\Games\eldenring.exe` fait passer au profil associé à
     `eldenring.exe` (rétroéclairage changé), retour au profil par défaut ensuite.
   - Autres bureaux (GNOME, Hyprland, Sway…) : non pris en charge pour l'instant ; repli propre
     (un avertissement, changement automatique désactivé, le reste fonctionne).
7. **Média et volume** : MPRIS (D-Bus, crate `zbus`) pour le morceau en cours / le LCD ; volume de la
   sortie du casque via PipeWire (`wpctl` ou API).
   **Fait (2026-10-17).**
   - MPRIS (`platform/linux/media.rs`, `zbus`) : lecteur en lecture, sinon le dernier montré, sinon un
     lecteur en pause ; titre, artistes, album, position, durée ; lecture / pause, suivant, précédent, stop.
   - Volume (`platform/linux/audio.rs`) : `pactl -f json list sinks` / `set-sink-volume` / `set-sink-mute`
     (pipewire-pulse). `pactl` met à `null` les textes non ASCII (« Stéréo analogique ») : la sortie est
     reconnue par `device.description` / `node.nick` / `device.product.name`.
   - Test : `cargo run -p g13-daemon --example media_probe -- "PRO X Wireless"` (avec un lecteur MPRIS
     ouvert) ; vérifié avec un faux lecteur (titre accentué, 2 artistes, position 61,5 / 245 s, commandes
     reçues) et sur le casque (40 % → 41 % + coupure → remis à 40 %).
   - L'égaliseur du pilote Logitech (`output_eq`) reste propre à Windows : sous Linux, c'est l'étape 8.
8. **Égaliseur du casque** : `g13-dsp` (biquads) existe déjà. Sous Linux, appliquer les mêmes 10 bandes
   par un **filter-chain PipeWire** (module `libpipewire-module-filter-chain`, filtres `bq_peaking`) sur la
   sortie du casque, regénéré quand le profil d'égaliseur change. Aucune élévation ni pilote.
   **Fait (2026-10-17).** Voir `docs/ARCHITECTURE.md` (égaliseur) et `platform/linux/eq.rs`.
   - Mesuré (sinus 1 kHz / 125 Hz à -23 dBFS, enregistrement *après* le filtre) : plat -23 ; 1 kHz +12
     → -23 à 1 kHz (préampli -12) et -34,9 à 125 Hz ; 125 Hz +6 → -23 / -29 ; réglages gardés après
     redémarrage du service ; désinstallé → -23 partout.
   - Piège de mesure : la règle du smart filter redirige aussi un enregistrement de la sortie vers
     l'entrée du filtre (avant égalisation) ; relier l'enregistreur à la main au moniteur de la sortie.
   - Piège systemd : `filter-chain.service` a `Also=pipewire.socket` ; ne jamais le `disable`.
   - Test sans bruit : `eq_probe install <sortie>` sur une sortie nulle
     (`pactl load-module module-null-sink …`), `eq_probe gains …`, `eq_probe uninstall`.
   - Reste pour l'étape 9 : textes de l'interface (l'installation ne demande pas de droits sous Linux).
9. **Interface Tauri** : icône de zone de notification (AppIndicator), démarrage automatique
   (`~/.config/autostart`), masquer les textes / boutons propres à Windows (installation de l'APO,
   « démarrage de Windows »…), overlay en jeu (au-dessus des jeux sous Wayland : vérifier ce que permet
   le compositeur).
   **Fait (2026-10-17)**, vérifié par l'utilisateur dans l'appli (Plasma 6, Wayland) :
   - Icône de notification (AppIndicator / SNI) et son menu : OK.
   - Démarrage automatique : `tauri-plugin-autostart` écrit `~/.config/autostart/G Nexus.desktop`
     (`Exec=… --autostart`) ; une version de développement y inscrit son propre chemin (la version
     publiée réinscrit le sien à chaque démarrage).
   - Textes : classe `os-linux` (voir ARCHITECTURE, Langues) ; versions Linux traduites pour : G13 non
     détecté (règles udev), installation de l'égaliseur (filtre PipeWire, sans droits), démarrage à
     l'ouverture de session, note de l'overlay, sortie audio introuvable (PipeWire), manette (sans
     ViGEmBus).
   - Overlay : placé, gardé au-dessus et activé par le script KWin (titre « G Nexus — overlay [pos] »).
   - Onglet Applications : fenêtres du bureau KDE (`plasmashell`, `krunner`…) écartées.
   - Souris en temps réel (MouseButtonSpy + injection uinput) : indicateur d'appui, clics, molette,
     boutons latéraux, DPI : OK. En mode « profils intégrés », la souris ne signale pas ses boutons
     (pas d'indicateur), comme sous Windows.
   - Non traité : détection de LGS (`lcore.exe`) inutile sous Linux (laissée telle quelle).
10. **Paquet** : AppImage ou paquet Arch (PKGBUILD) incluant les règles udev ; service utilisateur
    systemd seulement si utile (l'appli tourne déjà en zone de notification).
    **Fait (2026-10-17) : paquet Arch** `gnexus` (`deploy/linux/arch/`, voir son README). Compilé depuis
    l'archive de la version publiée (`--features tauri/custom-protocol`, `CARGO_TARGET_DIR=target`) ;
    installe `/usr/bin/gnexus`, le `.desktop` et les icônes, `70-gnexus-uaccess.rules` dans
    `/usr/lib/udev/rules.d` (rechargées par le hook de pacman), `uinput` au démarrage ; la règle RECORD
    reste facultative (`/usr/share/doc/gnexus/`, message après installation). Pas de service systemd :
    zone de notification + démarrage à l'ouverture de session (réglage de l'appli).
11. **AppImage, Bazzite** (systèmes immuables : `/usr` en lecture seule, pas de `pacman` / `dnf`).
    **Fait (2026-10-31)**, vérifié sur Bazzite 44 (KDE Plasma 6, Wayland) dans une VM VMware avec le G13.
    - Choix : AppImage plutôt qu'un `.rpm` (`rpm-ostree install` = couche sur l'image, déconseillée par
      Bazzite, redémarrage) ou un Flatpak (bac à sable : hidraw, uinput, KWin, PipeWire compliqués).
      Elle tourne aussi sur les autres distributions.
    - Construite sur **Ubuntu 22.04** (glibc ancienne), en local dans une distrobox, en CI par
      `.github/workflows/linux-appimage.yml` (jointe à chaque version publiée, ou à télécharger depuis
      l'onglet Actions). `crates/g13-gui/tauri.linux.conf.json` (fusionné par Tauri sous Linux seulement :
      Windows inchangé) : binaire `gnexus`, cible `appimage`. WebKitGTK, GTK et AppIndicator sont inclus
      (~81 Mo). Le crochet GTK de linuxdeploy ne force plus `GDK_BACKEND=x11` : Wayland natif.
    - **Accès aux appareils** (`platform/linux/access.rs`) : une AppImage ne peut pas installer de règles
      udev. G Nexus teste l'accès *réel* toutes les 5 s (nœuds hidraw de ses appareils, retrouvés par le
      périphérique USB parent comme le fait `ATTRS`, et `/dev/uinput`) et affiche alors un bandeau
      AUTORISER L'ACCÈS : `pkexec /bin/sh -c …` (environnement vidé) écrit les règles embarquées
      (`include_str!` de `deploy/linux`) dans `/etc/udev/rules.d` et `uinput.conf` dans
      `/etc/modules-load.d` (modifiables sur Bazzite), puis `udevadm trigger`. Si `/dev/uinput` manquait au
      démarrage, le moteur n'a pas pu démarrer : G Nexus redémarre seul. RECORD s'autorise à part
      (*Paramètres › Général › Accès aux appareils*).
    - Sur Bazzite, tout est déjà accessible sans nos règles : `/dev/uinput` par les règles Steam,
      les nœuds hidraw Logitech par celles d'OpenRGB (mode 0666). Aucun bandeau.
    - **Conteneur** (`/run/.containerenv`, `/.dockerenv`) : bandeau d'avertissement, pas d'installation.
      Retour d'un utilisateur de Bazzite qui lançait G Nexus dans une distrobox : démarrage automatique
      absent (le `.desktop` pointait vers un chemin du conteneur), fuseau horaire faux (`/etc/localtime`
      du conteneur), overlay mal placé. Les trois sont corrects avec l'AppImage lancée sur le système.
    - **Démarrage automatique** : `auto-launch` écrit `Exec=<chemin> --autostart` sans guillemets ; l'AppImage
      s'appelle `G Nexus_x.y.z_amd64.AppImage` (espace). `enable_autostart` (g13-gui) remet le chemin entre
      guillemets. Le plugin enregistre le chemin de l'AppImage (`$APPIMAGE`), pas celui du montage
      temporaire ; il est réinscrit à chaque lancement (AppImage déplacée par Gear Lever).
    - Vérifié par l'utilisateur : interface, G13, icône de notification, overlay à gauche / au centre /
      à droite, heure locale, démarrage automatique après redémarrage.

## Points d'attention

- Ne jamais faire tourner G Nexus et un autre pilote des mêmes appareils en même temps (Piper / libratbag,
  OpenRazer, Solaar pour la souris, HeadsetControl pour le casque) : un seul propriétaire par appareil.
- Le G13 sous Linux peut être pris par le pilote noyau `hid-lg-g15` : vérifier et le contourner (hidraw).
- La configuration utilisateur se transfère avec PARAMÈTRES › CONFIGURATION › EXPORTER (fichier
  `.g13backup`, qui contient aussi les profils de la souris, de l'égaliseur et les macros) puis IMPORTER
  sous Linux.
