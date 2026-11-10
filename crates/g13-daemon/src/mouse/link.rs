//! HID++ 2.0 over hidapi: request / reply on the receiver's (or the cable's)
//! long-report collection, and onboard memory read / write.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CString;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use logi_hidpp::features as f;
use logi_hidpp::{DIRECT, Message, Request};

const LOGITECH: u16 = 0x046D;
/// Normal reply timeout (a wireless mouse answers in a few ms).
pub const TIMEOUT: Duration = Duration::from_millis(400);
/// While looking for devices: most indices behind a receiver are empty.
const PROBE_TIMEOUT: Duration = Duration::from_millis(120);

/// One HID++ device: the open collection + its device index.
pub struct Link {
    pub dev: HidDevice,
    pub index: u8,
    pub timeout: Duration,
    /// HID++ software id of our requests (tools use another one than G13 Studio).
    pub sw_id: u8,
    /// Notifications (software id 0) received while waiting for a reply: kept for [`Link::event`].
    pub events: RefCell<VecDeque<Event>>,
}

/// A notification of the device: (feature index, function / event number, data).
pub type Event = (u8, u8, [u8; 16]);

impl Link {
    /// Send a request and wait for its own answer (other programs' replies are skipped).
    pub fn call(&self, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16], String> {
        let req = Request::new(self.index, feature_index, function, params).with_sw_id(self.sw_id);
        self.dev.write(&req.to_bytes()).map_err(|e| format!("écriture : {e}"))?;
        let deadline = Instant::now() + self.timeout;
        let mut buf = [0u8; 64];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("pas de réponse".into());
            }
            let n = self.dev.read_timeout(&mut buf, left.as_millis().max(1) as i32).map_err(|e| format!("lecture : {e}"))?;
            let Some(msg) = (n > 0).then(|| Message::parse(&buf[..n])).flatten() else { continue };
            self.keep_event(&msg);
            if !req.matches(&msg) {
                continue;
            }
            return match msg {
                Message::Reply { data, .. } => Ok(data),
                Message::Error { code, .. } => Err(format!("erreur HID++ {code:?}")),
                Message::Error10 { code, .. } => Err(format!("erreur HID++ 1.0 (0x{code:02X})")),
                Message::Other { .. } => continue,
            };
        }
    }

    fn keep_event(&self, msg: &Message) {
        if let Message::Reply { device, feature_index, function, sw_id: 0, data } = *msg
            && device == self.index
        {
            let mut q = self.events.borrow_mut();
            if q.len() < 256 {
                q.push_back((feature_index, function, data));
            }
        }
    }

    /// Next notification of the device, waiting up to `wait` for one.
    pub fn event(&self, wait: Duration) -> Option<Event> {
        if let Some(e) = self.events.borrow_mut().pop_front() {
            return Some(e);
        }
        let mut buf = [0u8; 64];
        let n = self.dev.read_timeout(&mut buf, wait.as_millis().max(1) as i32).ok()?;
        let msg = (n > 0).then(|| Message::parse(&buf[..n])).flatten()?;
        self.keep_event(&msg);
        self.events.borrow_mut().pop_front()
    }

    /// Index of feature `id`, if the device has it.
    pub fn feature(&self, id: u16) -> Option<u8> {
        let d = self.call(0, 0, &id.to_be_bytes()).ok()?;
        f::root_get_feature(&d).map(|r| r.index)
    }

    pub fn name(&self) -> Option<String> {
        let i = self.feature(f::DEVICE_NAME)?;
        let len = self.call(i, 0, &[]).ok()?[0] as usize;
        let mut bytes = Vec::new();
        while bytes.len() < len {
            let d = self.call(i, 1, &[bytes.len() as u8]).ok()?;
            bytes.extend_from_slice(&d[..(len - bytes.len()).min(16)]);
        }
        Some(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
    }

    /// Read a whole sector of the onboard memory (feature index `oi` = 0x8100).
    pub fn read_sector(&self, oi: u8, sector: u16, size: u16) -> Result<Vec<u8>, String> {
        let mut data = vec![0u8; size as usize];
        let mut offset = 0u16;
        while offset < size {
            // a read must stay inside the sector: the last one overlaps
            let at = offset.min(size - 16);
            let d = self.call(oi, 5, &[(sector >> 8) as u8, sector as u8, (at >> 8) as u8, at as u8])?;
            data[at as usize..at as usize + 16].copy_from_slice(&d);
            offset += 16;
        }
        Ok(data)
    }

    /// Write a whole sector (address, 16-byte chunks, end), then read it back
    /// and compare. Factory sectors (0x0100+) are refused.
    pub fn write_sector(&self, oi: u8, sector: u16, data: &[u8]) -> Result<(), String> {
        if sector >= 0x0100 {
            return Err("secteur d'usine : écriture refusée".into());
        }
        let len = data.len() as u16;
        self.call(oi, 6, &[(sector >> 8) as u8, sector as u8, 0, 0, (len >> 8) as u8, len as u8])?;
        for chunk in data.chunks(16) {
            self.call(oi, 7, chunk)?;
        }
        self.call(oi, 8, &[])?;
        let back = self.read_sector(oi, sector, len)?;
        if back != data {
            return Err(format!("secteur 0x{sector:04X} : la relecture ne correspond pas"));
        }
        Ok(())
    }
}

/// A mouse found on a receiver or a cable.
pub struct Mouse {
    pub link: Link,
    pub name: String,
    pub product_id: u16,
    /// Where it was found, to watch for it closely once it falls asleep.
    pub spot: Spot,
}

/// A HID++ collection and a device index on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pub path: CString,
    pub index: u8,
}

impl Spot {
    /// Open the spot for a ping. The receiver does not report a mouse waking up
    /// (no 0x41 notification: the LIGHTSPEED link stays up while it sleeps, measured),
    /// but a sleeping mouse does not answer and a woken one answers at once.
    pub fn open(&self, api: &HidApi) -> Option<Link> {
        let dev = api.open_path(&self.path).ok()?;
        Some(Link { dev, index: self.index, timeout: PROBE_TIMEOUT, sw_id: logi_hidpp::SW_ID, events: RefCell::default() })
    }
}

impl Link {
    /// HID++ 2.0 ping: is the device there and awake?
    pub fn ping(&self) -> bool {
        self.call(0, 1, &[0, 0, 0x5A]).is_ok_and(|d| d[0] >= 2)
    }
}

/// The mouse at `spot`, if it is there and awake (without searching the other receivers).
pub fn find_at(api: &HidApi, spot: &Spot) -> Option<Mouse> {
    let info = api.device_list().find(|d| d.path() == spot.path.as_c_str())?;
    let mut link = spot.open(api)?;
    if !link.ping() {
        return None;
    }
    link.timeout = TIMEOUT;
    // just woken, the mouse answered the ping but may miss the next request (measured):
    // try again at once rather than one search round later
    (0..3).find_map(|_| link.feature(f::ONBOARD_PROFILES))?;
    let name = link.name().unwrap_or_else(|| "souris".into());
    Some(Mouse { link, name, product_id: info.product_id(), spot: spot.clone() })
}

/// Every HID++ 2.0 device with onboard profiles (gaming mice) currently reachable.
pub fn find_mice(api: &HidApi) -> Vec<Mouse> {
    let mut out = Vec::new();
    for info in api.device_list().filter(|d| d.vendor_id() == LOGITECH) {
        if info.usage_page() != 0xFF00 || info.usage() != 0x0002 {
            continue;
        }
        let Ok(mut dev) = info.open_device(api) else { continue };
        for index in [DIRECT, 1, 2, 3, 4, 5, 6] {
            let mut link = Link { dev, index, timeout: PROBE_TIMEOUT, sw_id: logi_hidpp::SW_ID, events: RefCell::default() };
            let is_mouse = link.ping() && {
                link.timeout = TIMEOUT;
                link.feature(f::ONBOARD_PROFILES).is_some()
            };
            if is_mouse {
                let name = link.name().unwrap_or_else(|| "souris".into());
                let spot = Spot { path: info.path().to_owned(), index };
                out.push(Mouse { link, name, product_id: info.product_id(), spot });
                break;
            }
            dev = link.dev;
        }
    }
    out
}
