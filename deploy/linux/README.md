# Déploiement Linux (CachyOS / Arch, Bazzite, AppImage)

Le paquet Arch (`arch/`, voir `arch/README.md`) installe tout cela lui-même, et l'AppImage le propose
(bandeau « AUTORISER L'ACCÈS », mot de passe administrateur demandé une fois : les fichiers vont dans
`/etc`, modifiable même sur Bazzite). Les commandes ci-dessous ne servent qu'en compilant à la main.

Accès sans root aux trois appareils (G13, G502 X PLUS câblée ou LIGHTSPEED, dongle du casque
PRO X Wireless) et à `/dev/uinput`, pour l'utilisateur connecté :

```sh
sudo install -m644 70-gnexus-uaccess.rules /etc/udev/rules.d/
sudo install -m644 uinput.conf /etc/modules-load.d/
sudo modprobe uinput
sudo udevadm control --reload && sudo udevadm trigger
```

Pas besoin de se déconnecter : les ACL s'appliquent dès le `trigger` (rebrancher l'appareil si besoin).

Optionnel, pour enregistrer des macros (bouton RECORD) : la lecture des claviers et souris pour la
session active. Contrepartie : tout programme de l'utilisateur peut alors lire la frappe (comme sous
X11). Sans elle, seul RECORD est indisponible.

```sh
sudo install -m644 71-gnexus-record.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=input
```

Variante par groupe `input` (si `uaccess` ne convient pas, par ex. session sans systemd-logind) :
installer `99-gnexus-input-group.rules` à la place, puis `sudo usermod -aG input "$USER"` et se
reconnecter. Moins restrictive : le groupe `input` lit *tous* les périphériques d'entrée.

Vérification :

```sh
getfacl /dev/hidraw* /dev/uinput | grep -B4 "user:$USER"   # uaccess : ACL au nom de l'utilisateur
g13d --dry-run -v                                           # G13
hidpp-probe                                                 # souris et casque (lecture seule)
```

Le pilote noyau `hid-lg-g15` se lie au G13 : il ne gêne pas G Nexus, qui passe par hidraw
(voir docs/LINUX_PORT.md, étape 2).

Ne pas faire tourner en même temps un autre logiciel sur ces appareils (Solaar, Piper / libratbag,
HeadsetControl) : un seul propriétaire par appareil.
