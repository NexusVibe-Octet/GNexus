# Paquet Arch / CachyOS (`gnexus`)

Ce que le paquet installe :

| Fichier | Rôle |
|---|---|
| `/usr/bin/gnexus` | l'application (binaire `g13-studio` du crate `g13-gui`, pages intégrées : `tauri/custom-protocol`) |
| `/usr/share/applications/gnexus.desktop`, icônes `hicolor` | entrée du menu ; l'app-id Wayland est le nom du binaire (`gnexus`) |
| `/usr/lib/udev/rules.d/70-gnexus-uaccess.rules` | accès sans root au G13, à la souris, au dongle du casque et à `/dev/uinput` |
| `/usr/lib/modules-load.d/gnexus-uinput.conf` | `uinput` chargé au démarrage |
| `/usr/share/doc/gnexus/71-gnexus-record.rules` | **facultatif** (RECORD) : à copier soi-même dans `/etc/udev/rules.d/` |

Les règles udev sont rechargées par le hook de pacman (`35-systemd-udev-reload.hook`). Aucun service
systemd : l'application vit dans la zone de notification et se lance à l'ouverture de session si on le
demande dans ses paramètres (`~/.config/autostart/G Nexus.desktop`).

## Construire depuis une version publiée

```sh
cd deploy/linux/arch
updpkgsums          # une fois l'archive v1.1.4 publiée sur GitHub (remplace SKIP)
makepkg -si
```

Rust vient de `rustup` ? `makepkg` réclame le paquet `cargo` : `makepkg -d -si` saute la vérification
des dépendances (les autres dépendances doivent alors déjà être installées).

## Construire avant la publication (sources locales)

`makepkg` prend l'archive si elle est déjà là, sous le nom attendu :

```sh
B=$(mktemp -d) && cp deploy/linux/arch/{PKGBUILD,gnexus.install} "$B"
tar --exclude=./target --exclude=./.git --transform 's,^\.,GNexus-1.1.4,' -czf "$B/gnexus-1.1.4.tar.gz" .
cd "$B" && makepkg -d -f
sudo pacman -U gnexus-1.1.4-1-x86_64.pkg.tar.zst
```

## Publier

- Joindre `gnexus-<version>-1-x86_64.pkg.tar.zst` à la Release GitHub (installation :
  `sudo pacman -U gnexus-….pkg.tar.zst`).
- AUR (facultatif) : le `PKGBUILD` (avec les vraies sommes, `updpkgsums`), `gnexus.install` et le
  `.SRCINFO` (`makepkg --printsrcinfo > .SRCINFO`) dans le dépôt AUR `gnexus`.
- Nouvelle version : `pkgver` ici, `version` dans `crates/g13-gui/tauri.conf.json` et `Cargo.toml`.
