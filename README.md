# G Nexus

*[Version française](README.fr.md)*

G Nexus replaces Logitech Gaming Software and G HUB for three Logitech devices, on **Windows** and **Linux**:

- **G13 Advanced Gameboard** (keyboard)
- **G502 X PLUS** mouse (wired or LIGHTSPEED receiver)
- **PRO X Wireless** headset

It runs in the notification area, needs no Logitech software, and keeps working without an internet connection.

<p align="center"><img src="docs/screenshots/g13.png" width="820" alt="G Nexus — G13 page"></p>

<a href="https://ko-fi.com/W4F728ZFSH"><img height="36" src="https://storage.ko-fi.com/cdn/kofi6.png?v=6" alt="Buy Me a Coffee at ko-fi.com"></a>

## Screenshots

<table>
  <tr>
    <td align="center"><a href="docs/screenshots/mouse.png"><img src="docs/screenshots/mouse.png" width="400" alt="G502 X PLUS: buttons, DPI, lighting"></a><br><sub>G502 X PLUS: buttons, DPI, lighting</sub></td>
    <td align="center"><a href="docs/screenshots/headset.png"><img src="docs/screenshots/headset.png" width="400" alt="PRO X Wireless: battery, volume, sleep"></a><br><sub>PRO X Wireless: battery, volume, sleep</sub></td>
  </tr>
  <tr>
    <td align="center"><a href="docs/screenshots/equalizer.png"><img src="docs/screenshots/equalizer.png" width="400" alt="10-band equalizer with per-application profiles"></a><br><sub>10-band equalizer with per-application profiles</sub></td>
    <td align="center"><a href="docs/screenshots/key-editor.png"><img src="docs/screenshots/key-editor.png" width="400" alt="Key editor: tap, hold, tap-tap, tap-hold"></a><br><sub>Key editor: tap, hold, tap-tap, tap-hold</sub></td>
  </tr>
  <tr>
    <td align="center"><a href="docs/screenshots/macros.png"><img src="docs/screenshots/macros.png" width="400" alt="Macros: editor and RECORD"></a><br><sub>Macros: editor and RECORD</sub></td>
    <td align="center"><a href="docs/screenshots/color.png"><img src="docs/screenshots/color.png" width="400" alt="Backlight colour per bank"></a><br><sub>Backlight colour per bank</sub></td>
  </tr>
  <tr>
    <td align="center" colspan="2"><a href="docs/screenshots/overlay-in-game.webp"><img src="docs/screenshots/overlay-in-game.webp" width="820" alt="In-game overlay: the G13 over the game, on the game's profile (G25 or Esc to go back)"></a><br><sub>In-game overlay: the G13 over the game, on the game's profile (G25 or Esc to go back)</sub></td>
  </tr>
</table>

## Features

**G13**
- Every key remappable: keys and shortcuts, mouse buttons and wheel, macros, multi-state keys (tap / hold / double tap)
- Three banks (M1–M3) with their own backlight colour, profiles per application (switched automatically with the game in the foreground)
- Macros: editor, library, recording with the RECORD (MR) key
- Stick as four keys, or as the analog stick of a virtual Xbox 360 controller
- LCD screen applets: clock, system monitor, now playing (media controls), profile picker
- In-game overlay to edit keys without leaving the game
- Import / export of Logitech Gaming Software profiles (`.xml`)

**G502 X PLUS**
- DPI levels, report rate, lighting
- Onboard profiles stored in the mouse, or real-time mode with one profile per game
- Button remapping and mouse macros

**PRO X Wireless**
- Battery, sidetone, automatic sleep, volume
- G Nexus's own 10-band equalizer, with profiles per application

**And also**: automatic backups, export / import of the whole configuration (`.g13backup`), interface in French, English, German, Italian and Spanish.

## Download

Installers are attached to each [release](../../releases): Windows installer, Linux AppImage and Arch / CachyOS package.

### Windows

1. Run the installer (`G Nexus_x.y.z_x64-setup.exe`). WebView2 is required (included in Windows 11).
2. Close Logitech Gaming Software: it captures the G13's keys. Keep G HUB closed while G Nexus drives the mouse or the headset.
3. Optional: the [ViGEmBus](https://github.com/nefarius/ViGEmBus) driver for the stick's gamepad mode.
4. The equalizer is installed from the Headset page (Windows asks for administrator rights once).

### Linux

Tested on CachyOS (Arch) and **Bazzite** (Fedora Atomic), KDE Plasma 6, Wayland, PipeWire. Automatic profile switching and the in-game overlay need **KDE Plasma**; everything else works on any desktop.

1. **AppImage** (Bazzite, Fedora, Ubuntu, Mint… and any other distribution): download `gnexus_x.y.z_x86_64.AppImage` from the release, make it executable (right click › Properties › Executable, or `chmod +x`) and run it. Nothing is installed into the system and no reboot is needed. On Bazzite, *Gear Lever* (preinstalled) can add it to the application menu and keep it up to date.
   - **Run the AppImage directly on the system, not inside a distrobox / toolbox**: in a container, starting at login, the LCD clock and the overlay misbehave (G Nexus shows a banner about it).
2. **Arch / CachyOS**: install the package attached to the release instead, `sudo pacman -U gnexus-x.y.z-1-x86_64.pkg.tar.zst` (or build it with the [`PKGBUILD`](deploy/linux/arch)). It installs the udev rules and loads `uinput`; plug the devices in again afterwards.
3. **Device access without root** comes from the udev rules in [`deploy/linux`](deploy/linux). With the AppImage, when a plugged-in device or the virtual keyboard (`/dev/uinput`) cannot be opened, G Nexus shows an **ALLOW ACCESS** banner: the administrator password is asked once and the rules are copied to `/etc` (writable, even on Bazzite). On Bazzite the devices are often already accessible (rules shipped by the system) and nothing is asked. Macro recording (RECORD key) is allowed separately: *Settings › General › Device access*.
4. Do not run another program driving the same devices at the same time (Solaar, Piper / libratbag, HeadsetControl).
5. The equalizer is a PipeWire filter placed in front of the headset's output: no root rights, and the default output does not change.

Your Windows configuration can be moved over: *Settings › Configuration › Export* on Windows, then *Import* on Linux.

## Building from source

Rust (stable) is required. The workspace is `cargo test --workspace`, `cargo clippy --workspace --all-targets`.

- **Linux** (Arch packages): `webkit2gtk-4.1 gtk3 libayatana-appindicator hidapi systemd-libs base-devel`, then
  `CARGO_TARGET_DIR=target cargo build --release -p g13-gui` (binary `target/release/g13-studio`).
  `.cargo/config.toml` holds a build directory specific to the author's Windows machine: `CARGO_TARGET_DIR` overrides it.
- **AppImage**: on Ubuntu 22.04 (a distrobox is enough, e.g. on Bazzite), packages
  `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev libudev-dev libfuse2 patchelf`,
  then from `crates/g13-gui`: `CARGO_TARGET_DIR=../../target npx @tauri-apps/cli@2 build --bundles appimage`
  (or `cargo tauri build --bundles appimage`). The [`linux-appimage.yml`](.github/workflows/linux-appimage.yml) workflow
  builds it and attaches it to every published release.
- **Windows** (GNU toolchain): from `crates/g13-gui`, `npx @tauri-apps/cli@2 build` builds the NSIS installer.

Technical documentation (in French): [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/LINUX_PORT.md`](docs/LINUX_PORT.md).

## Credits

Design inspired by the YouTube channel [JaspMakes](https://www.youtube.com/watch?v=315USgAl2Ks).

## Disclaimer

G Nexus is not affiliated with Logitech. Logitech, G13, G502 and PRO X are trademarks of Logitech.
It talks to the devices with their own protocols (HID, HID++ 2.0); use it at your own risk.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
