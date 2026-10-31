//! Device access without root: can G Nexus open the G13, the mouse, the headset dongle and
//! `/dev/uinput`, and install its udev rules (`pkexec`, once) when it cannot.
//!
//! The Arch package installs the rules itself; an AppImage cannot, so the app checks the
//! real access (not the presence of its rule files: Bazzite's OpenRGB rules, for instance,
//! already open the Logitech devices) and offers the installation. The rules go to `/etc`,
//! which stays writable on image-based systems (Bazzite, Fedora Atomic).

use serde::Serialize;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

const UACCESS_RULES: &str = include_str!("../../../../../deploy/linux/70-gnexus-uaccess.rules");
const RECORD_RULES: &str = include_str!("../../../../../deploy/linux/71-gnexus-record.rules");
const UINPUT_CONF: &str = include_str!("../../../../../deploy/linux/uinput.conf");

/// Logitech devices G Nexus drives (USB product id → name shown in the window).
const DEVICES: &[(u16, &str)] = &[
    (0xc21c, "G13"),
    (0xc095, "G502 X PLUS"),
    (0xc547, "G502 X PLUS (LIGHTSPEED)"),
    (0x0aba, "PRO X Wireless"),
];
const LOGITECH: u16 = 0x046d;

const RECORD_NAME: &str = "71-gnexus-record.rules";
const GROUP_NAME: &str = "99-gnexus-input-group.rules";

#[derive(Debug, Clone, Default, Serialize)]
pub struct AccessStatus {
    /// Devices plugged in that G Nexus cannot open (one name per device).
    pub blocked: Vec<String>,
    /// `/dev/uinput` writable: virtual keyboard, mouse and gamepad.
    pub uinput: bool,
    /// Macro recording (RECORD) allowed: G Nexus's optional rule, or the `input` group.
    pub record: bool,
    /// Running inside a container (distrobox, toolbox): the rules would not reach the system.
    pub container: bool,
}

impl AccessStatus {
    /// Something G Nexus needs is missing (the optional RECORD rule does not count).
    pub fn missing(&self) -> bool {
        !self.blocked.is_empty() || !self.uinput
    }
}

/// What the installation changed.
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    /// `/dev/uinput` was not usable when G Nexus started: its engine is not running, a
    /// restart is needed.
    pub restart: bool,
}

pub fn status() -> AccessStatus {
    AccessStatus {
        blocked: blocked_devices(),
        uinput: writable(Path::new("/dev/uinput")),
        record: record_allowed(),
        container: in_container(),
    }
}

/// Inside a container (podman / docker: distrobox, toolbox).
pub fn in_container() -> bool {
    Path::new("/run/.containerenv").exists() || Path::new("/.dockerenv").exists()
}

fn writable(p: &Path) -> bool {
    OpenOptions::new().read(true).write(true).open(p).is_ok()
}

/// Our devices' hidraw nodes that cannot be opened. The mouse behind its LIGHTSPEED
/// receiver has its own node (`046D:4099`, from `hid-logitech-dj`): it is recognised by the
/// USB device it hangs from, as the udev rules do (`ATTRS`).
fn blocked_devices() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hidraw") else { return out };
    for entry in dir.flatten() {
        let Some(name) = usb_product(&entry.path().join("device")) else { continue };
        let node = Path::new("/dev").join(entry.file_name());
        if !writable(&node) && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Name of the G Nexus device that `dev` (a sysfs device) belongs to, if any.
fn usb_product(dev: &Path) -> Option<&'static str> {
    let mut p: PathBuf = std::fs::canonicalize(dev).ok()?;
    loop {
        if let (Some(v), Some(id)) = (read_hex(&p.join("idVendor")), read_hex(&p.join("idProduct"))) {
            return (v == LOGITECH).then(|| DEVICES.iter().find(|(pid, _)| *pid == id).map(|(_, n)| *n)).flatten();
        }
        if !p.pop() || p.as_os_str().len() <= "/sys".len() {
            return None;
        }
    }
}

fn read_hex(p: &Path) -> Option<u16> {
    u16::from_str_radix(std::fs::read_to_string(p).ok()?.trim(), 16).ok()
}

fn rule_installed(name: &str) -> bool {
    ["/etc/udev/rules.d", "/usr/lib/udev/rules.d", "/lib/udev/rules.d"].iter().any(|d| Path::new(d).join(name).exists())
}

fn record_allowed() -> bool {
    if rule_installed(RECORD_NAME) || rule_installed(GROUP_NAME) {
        return true;
    }
    // member of the `input` group: reads every keyboard and mouse
    let groups = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let gid = std::fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("input:").and_then(|r| r.split(':').nth(1)).map(str::to_string));
    match gid {
        Some(gid) => groups
            .lines()
            .find_map(|l| l.strip_prefix("Groups:"))
            .is_some_and(|g| g.split_whitespace().any(|x| x == gid)),
        None => false,
    }
}

/// Root part of the installation, run by `pkexec /bin/sh -c`. `$1` uaccess rules, `$2`
/// uinput.conf, `$3` RECORD rules (empty: not installed).
const SCRIPT: &str = r#"set -e
install -d -m755 /etc/udev/rules.d /etc/modules-load.d
printf '%s' "$1" > /etc/udev/rules.d/70-gnexus-uaccess.rules
printf '%s' "$2" > /etc/modules-load.d/gnexus-uinput.conf
chmod 644 /etc/udev/rules.d/70-gnexus-uaccess.rules /etc/modules-load.d/gnexus-uinput.conf
if [ -n "$3" ]; then
    printf '%s' "$3" > /etc/udev/rules.d/71-gnexus-record.rules
    chmod 644 /etc/udev/rules.d/71-gnexus-record.rules
fi
modprobe uinput 2>/dev/null || true
udevadm control --reload
udevadm trigger --action=change --subsystem-match=hidraw --subsystem-match=usb --subsystem-match=misc
if [ -n "$3" ]; then udevadm trigger --action=change --subsystem-match=input; fi
udevadm settle --timeout=10 || true
"#;

/// Install G Nexus's udev rules and load `uinput` at boot, with the administrator password
/// asked once by polkit (`pkexec`). `record`: also allow macro recording (RECORD).
pub fn install(record: bool) -> Result<InstallResult, String> {
    if in_container() {
        return Err("G Nexus tourne dans un conteneur : les règles n'atteindraient pas le système".into());
    }
    let before = status();
    let out = Command::new("pkexec")
        .args(["/bin/sh", "-c", SCRIPT, "gnexus-setup", UACCESS_RULES, UINPUT_CONF])
        .arg(if record { RECORD_RULES } else { "" })
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .envs(std::env::vars().filter(|(k, _)| matches!(k.as_str(), "DISPLAY" | "WAYLAND_DISPLAY" | "XAUTHORITY" | "XDG_RUNTIME_DIR" | "DBUS_SESSION_BUS_ADDRESS" | "LANG" | "LC_ALL")))
        .output()
        .map_err(|e| format!("pkexec introuvable ({e}) : installez les règles à la main (deploy/linux/README.md)"))?;
    match out.status.code() {
        Some(0) => {}
        // dialog closed, or wrong password three times
        Some(126) | Some(127) => return Err("autorisation refusée ou annulée".into()),
        _ => {
            let msg = String::from_utf8_lossy(&out.stderr);
            return Err(format!("échec de l'installation : {}", msg.trim()));
        }
    }
    // ACLs are applied by udev a moment after the trigger
    for _ in 0..20 {
        if !status().missing() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(InstallResult { restart: !before.uinput })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_rules_cover_every_device() {
        for (pid, _) in DEVICES {
            assert!(UACCESS_RULES.contains(&format!("{pid:04x}")), "{pid:04x} missing from 70-gnexus-uaccess.rules");
        }
        assert!(UACCESS_RULES.contains("uinput"));
        assert!(UINPUT_CONF.lines().any(|l| l.trim() == "uinput"));
        assert!(RECORD_RULES.contains("ID_INPUT_KEYBOARD"));
    }

    #[test]
    fn status_does_not_panic() {
        let s = status();
        let _ = s.missing();
    }
}
