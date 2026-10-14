//! Network interface throughput from `/proc/net/dev`.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::disk::HISTORY;
use crate::sysfs::{read_trim, read_u64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ethernet,
    Wireless,
    Bond,
    Virtual,
    Loopback,
}

#[derive(Clone, Copy, Default)]
struct Raw {
    rx_bytes: u64,
    rx_packets: u64,
    rx_errs: u64,
    rx_drop: u64,
    tx_bytes: u64,
    tx_packets: u64,
    tx_errs: u64,
    tx_drop: u64,
}

pub struct Iface {
    pub name: String,
    pub kind: Kind,
    pub state: String,
    pub speed_mbps: Option<u64>,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_pps: f64,
    pub tx_pps: f64,
    pub errors: u64,
    pub drops: u64,
    pub total_rx: u64,
    pub total_tx: u64,
    pub history: VecDeque<(f64, f64)>,
    prev: Option<Raw>,
    seen: bool,
}

pub struct NetMonitor {
    pub ifaces: Vec<Iface>,
    last: Option<Instant>,
}

impl NetMonitor {
    pub fn new() -> Self {
        let mut m = Self { ifaces: Vec::new(), last: None };
        m.refresh();
        m
    }

    pub fn refresh(&mut self) {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);

        for i in &mut self.ifaces {
            i.seen = false;
        }
        for (name, raw) in read_net_dev() {
            let idx = match self.ifaces.iter().position(|i| i.name == name) {
                Some(i) => i,
                None => {
                    self.ifaces.push(Iface::probe(&name));
                    self.ifaces.len() - 1
                }
            };
            self.ifaces[idx].update(raw, dt);
        }
        self.ifaces.retain(|i| i.seen);
        self.ifaces.sort_by(|a, b| (a.kind as u8, &a.name).cmp(&(b.kind as u8, &b.name)));
    }
}

impl Iface {
    fn probe(name: &str) -> Self {
        let sys = Path::new("/sys/class/net").join(name);
        let kind = if name == "lo" {
            Kind::Loopback
        } else if sys.join("bonding").exists() {
            Kind::Bond
        } else if sys.join("wireless").exists() || sys.join("phy80211").exists() {
            Kind::Wireless
        } else if sys.join("device").exists() {
            Kind::Ethernet
        } else {
            Kind::Virtual
        };
        Self {
            name: name.to_string(),
            kind,
            state: String::new(),
            speed_mbps: None,
            rx_bps: 0.0,
            tx_bps: 0.0,
            rx_pps: 0.0,
            tx_pps: 0.0,
            errors: 0,
            drops: 0,
            total_rx: 0,
            total_tx: 0,
            history: VecDeque::with_capacity(HISTORY),
            prev: None,
            seen: true,
        }
    }

    fn update(&mut self, raw: Raw, dt: f64) {
        self.seen = true;
        let sys = Path::new("/sys/class/net").join(&self.name);
        self.state = read_trim(sys.join("operstate")).unwrap_or_else(|| "?".into());
        // Reading `speed` fails (EINVAL) for wireless/down links; the kernel
        // reports -1 (as u32 max) for unknown.
        self.speed_mbps = read_u64(sys.join("speed")).filter(|&s| s > 0 && s < u32::MAX as u64);
        self.errors = raw.rx_errs + raw.tx_errs;
        self.drops = raw.rx_drop + raw.tx_drop;
        self.total_rx = raw.rx_bytes;
        self.total_tx = raw.tx_bytes;
        if let (Some(p), true) = (self.prev, dt > 0.0) {
            let d = |a: u64, b: u64| a.saturating_sub(b) as f64 / dt;
            self.rx_bps = d(raw.rx_bytes, p.rx_bytes);
            self.tx_bps = d(raw.tx_bytes, p.tx_bytes);
            self.rx_pps = d(raw.rx_packets, p.rx_packets);
            self.tx_pps = d(raw.tx_packets, p.tx_packets);
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back((self.rx_bps, self.tx_bps));
        }
        self.prev = Some(raw);
    }
}

fn read_net_dev() -> HashMap<String, Raw> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string("/proc/net/dev") else { return out };
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let f: Vec<u64> = rest.split_whitespace().map(|v| v.parse().unwrap_or(0)).collect();
        if f.len() < 16 {
            continue;
        }
        out.insert(
            name.trim().to_string(),
            Raw {
                rx_bytes: f[0],
                rx_packets: f[1],
                rx_errs: f[2],
                rx_drop: f[3],
                tx_bytes: f[8],
                tx_packets: f[9],
                tx_errs: f[10],
                tx_drop: f[11],
            },
        );
    }
    out
}
