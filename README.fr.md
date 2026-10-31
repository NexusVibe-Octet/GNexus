# G Nexus

*[English version](README.md)*

G Nexus remplace Logitech Gaming Software et G HUB pour trois appareils Logitech, sous **Windows** et **Linux** :

- clavier **G13 Advanced Gameboard**
- souris **G502 X PLUS** (filaire ou récepteur LIGHTSPEED)
- casque **PRO X Wireless**

Il tourne dans la zone de notification, sans logiciel Logitech, et fonctionne sans connexion internet.

<p align="center"><img src="docs/screenshots/g13.png" width="820" alt="G Nexus — page du G13"></p>

<a href="https://ko-fi.com/W4F728ZFSH"><img height="36" src="https://storage.ko-fi.com/cdn/kofi6.png?v=6" alt="Buy Me a Coffee at ko-fi.com"></a>

## Captures d'écran

<table>
  <tr>
    <td align="center"><a href="docs/screenshots/mouse.png"><img src="docs/screenshots/mouse.png" width="400" alt="G502 X PLUS : boutons, DPI, éclairage"></a><br><sub>G502 X PLUS : boutons, DPI, éclairage</sub></td>
    <td align="center"><a href="docs/screenshots/headset.png"><img src="docs/screenshots/headset.png" width="400" alt="PRO X Wireless : batterie, volume, mise en veille"></a><br><sub>PRO X Wireless : batterie, volume, mise en veille</sub></td>
  </tr>
  <tr>
    <td align="center"><a href="docs/screenshots/equalizer.png"><img src="docs/screenshots/equalizer.png" width="400" alt="Égaliseur 10 bandes avec profils par application"></a><br><sub>Égaliseur 10 bandes avec profils par application</sub></td>
    <td align="center"><a href="docs/screenshots/key-editor.png"><img src="docs/screenshots/key-editor.png" width="400" alt="Éditeur de touche : tap, hold, tap-tap, tap-hold"></a><br><sub>Éditeur de touche : tap, hold, tap-tap, tap-hold</sub></td>
  </tr>
  <tr>
    <td align="center"><a href="docs/screenshots/macros.png"><img src="docs/screenshots/macros.png" width="400" alt="Macros : éditeur et RECORD"></a><br><sub>Macros : éditeur et RECORD</sub></td>
    <td align="center"><a href="docs/screenshots/color.png"><img src="docs/screenshots/color.png" width="400" alt="Couleur du rétroéclairage par menu"></a><br><sub>Couleur du rétroéclairage par menu</sub></td>
  </tr>
  <tr>
    <td align="center" colspan="2"><a href="docs/screenshots/overlay-in-game.webp"><img src="docs/screenshots/overlay-in-game.webp" width="820" alt="Overlay en jeu : le G13 par-dessus le jeu, sur le profil du jeu (G25 ou Échap pour y revenir)"></a><br><sub>Overlay en jeu : le G13 par-dessus le jeu, sur le profil du jeu (G25 ou Échap pour y revenir)</sub></td>
  </tr>
</table>

## Fonctionnalités

**G13**
- Toutes les touches réaffectables : touches et raccourcis, boutons et molette de souris, macros, touches multi-états (appui court / long / double)
- Trois banques (M1 à M3) avec leur couleur de rétroéclairage, profils par application (changés automatiquement selon le jeu au premier plan)
- Macros : éditeur, bibliothèque, enregistrement avec la touche RECORD (MR)
- Stick en quatre touches, ou joystick analogique d'une manette Xbox 360 virtuelle
- Applets de l'écran LCD : horloge, moniteur système, lecture en cours (commandes média), choix du profil
- Overlay en jeu pour modifier les touches sans quitter le jeu
- Import / export des profils Logitech Gaming Software (`.xml`)

**G502 X PLUS**
- Paliers DPI, fréquence, éclairage
- Profils intégrés à la souris, ou mode temps réel avec un profil par jeu
- Réaffectation des boutons et macros souris

**PRO X Wireless**
- Batterie, retour micro, mise en veille automatique, volume
- Égaliseur 10 bandes propre à G Nexus, avec des profils par application

**Et aussi** : sauvegardes automatiques, export / import de toute la configuration (`.g13backup`), interface en français, anglais, allemand, italien et espagnol.

## Téléchargement

Les installateurs sont joints à chaque [version publiée](../../releases) : installateur Windows, AppImage Linux et paquet Arch / CachyOS.

### Windows

1. Lancez l'installateur (`G Nexus_x.y.z_x64-setup.exe`). WebView2 est nécessaire (présent dans Windows 11).
2. Fermez Logitech Gaming Software : il capte les touches du G13. Gardez G HUB fermé pendant que G Nexus pilote la souris ou le casque.
3. Facultatif : le pilote [ViGEmBus](https://github.com/nefarius/ViGEmBus) pour le mode manette du stick.
4. L'égaliseur s'installe depuis la page Casque (Windows demande une fois les droits administrateur).

### Linux

Testé sous CachyOS (Arch) et **Bazzite** (Fedora Atomic), KDE Plasma 6, Wayland, PipeWire. Le changement automatique de profil et l'overlay en jeu demandent **KDE Plasma** ; tout le reste fonctionne sur n'importe quel bureau.

1. **AppImage** (Bazzite, Fedora, Ubuntu, Mint… et toute autre distribution) : téléchargez `gnexus_x.y.z_x86_64.AppImage` depuis la version publiée, rendez-le exécutable (clic droit › Propriétés › Exécutable, ou `chmod +x`) et lancez-le. Rien n'est installé dans le système et aucun redémarrage n'est nécessaire. Sur Bazzite, *Gear Lever* (préinstallé) peut l'ajouter au menu des applications et le mettre à jour.
   - **Lancez l'AppImage directement sur le système, pas dans une distrobox / toolbox** : dans un conteneur, le démarrage à l'ouverture de session, l'heure du LCD et l'overlay fonctionnent mal (G Nexus le signale par un bandeau).
2. **Arch / CachyOS** : installez plutôt le paquet joint à la version publiée, `sudo pacman -U gnexus-x.y.z-1-x86_64.pkg.tar.zst` (ou construisez-le avec le [`PKGBUILD`](deploy/linux/arch)). Il installe les règles udev et charge `uinput` ; rebranchez ensuite les appareils.
3. **Accès aux appareils sans droits root** : il vient des règles udev de [`deploy/linux`](deploy/linux). Avec l'AppImage, si un appareil branché ou le clavier virtuel (`/dev/uinput`) n'est pas accessible, G Nexus affiche un bandeau **AUTORISER L'ACCÈS** : le mot de passe administrateur est demandé une seule fois et les règles sont copiées dans `/etc` (modifiable, même sur Bazzite). Sur Bazzite, les appareils sont souvent déjà accessibles (règles fournies par le système) et rien n'est demandé. L'enregistrement des macros (touche RECORD) s'autorise à part : *Paramètres › Général › Accès aux appareils*.
4. Ne faites pas tourner en même temps un autre programme qui pilote les mêmes appareils (Solaar, Piper / libratbag, HeadsetControl).
5. L'égaliseur est un filtre PipeWire placé devant la sortie du casque : aucun droit root, et la sortie par défaut ne change pas.

Votre configuration Windows se transfère : *Paramètres › Configuration › Exporter* sous Windows, puis *Importer* sous Linux.

## Compiler depuis les sources

Rust (stable) est nécessaire. Tests : `cargo test --workspace` ; lint : `cargo clippy --workspace --all-targets`.

- **Linux** (paquets Arch) : `webkit2gtk-4.1 gtk3 libayatana-appindicator hidapi systemd-libs base-devel`, puis
  `CARGO_TARGET_DIR=target cargo build --release -p g13-gui` (binaire `target/release/g13-studio`).
  `.cargo/config.toml` contient un dossier de compilation propre à la machine Windows de l'auteur : `CARGO_TARGET_DIR` le remplace.
- **AppImage** : sur Ubuntu 22.04 (une distrobox suffit, par exemple sous Bazzite), paquets
  `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev libudev-dev libfuse2 patchelf`,
  puis depuis `crates/g13-gui` : `CARGO_TARGET_DIR=../../target npx @tauri-apps/cli@2 build --bundles appimage`
  (ou `cargo tauri build --bundles appimage`). Le workflow [`linux-appimage.yml`](.github/workflows/linux-appimage.yml)
  la construit et la joint à chaque version publiée.
- **Windows** (toolchain GNU) : depuis `crates/g13-gui`, `npx @tauri-apps/cli@2 build` produit l'installateur NSIS.

Documentation technique : [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/LINUX_PORT.md`](docs/LINUX_PORT.md).

## Crédits

Design inspiré par la chaîne YouTube [JaspMakes](https://www.youtube.com/watch?v=315USgAl2Ks).

## Avertissement

G Nexus n'est pas affilié à Logitech. Logitech, G13, G502 et PRO X sont des marques de Logitech.
Il dialogue avec les appareils par leurs propres protocoles (HID, HID++ 2.0) ; utilisation à vos risques.

## Licence

Au choix : [licence Apache 2.0](LICENSE-APACHE) ou [licence MIT](LICENSE-MIT).
