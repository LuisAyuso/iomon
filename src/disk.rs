//! Block-device I/O statistics.
//!
//! Stats come from `/proc/diskstats`, i.e. the block layer underneath every
//! filesystem, so they work the same for ext4, btrfs, xfs, zfs, etc. and
//! report what each physical drive actually does (including RAID overhead
//! such as mirrored writes and parity).

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::sysfs::{list_dir, physical_disks_of, read_trim, read_u64};

pub const HISTORY: usize = 240;
/// `/proc/diskstats` always counts in 512-byte sectors, regardless of the
/// device's logical block size.
const SECTOR: f64 = 512.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A real drive (NVMe, SATA/SAS, USB, MMC, virtio...).
    Disk,
    /// Linux software RAID (mdadm).
    Md,
    /// Device-mapper: LVM, LUKS, dm-raid...
    Dm,
    /// zram, loop, nbd, ...
    Virtual,
}

#[derive(Clone, Copy, Default)]
struct Raw {
    reads: u64,
    sectors_read: u64,
    read_ms: u64,
    writes: u64,
    sectors_written: u64,
    write_ms: u64,
    io_ticks: u64,
}

#[derive(Clone, Copy, Default)]
pub struct Rates {
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    /// Average time per completed request (ms).
    pub await_ms: f64,
    /// Percentage of time the device had I/O in flight.
    pub util: f64,
}

pub struct Disk {
    pub name: String,
    pub kind: Kind,
    pub model: String,
    pub media: &'static str,
    pub size: u64,
    pub rates: Rates,
    pub total_read: u64,
    pub total_written: u64,
    pub history: VecDeque<(f64, f64)>,
    /// Names of arrays this disk belongs to.
    pub arrays: Vec<String>,
    prev: Option<Raw>,
    seen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayKind {
    Md,
    Btrfs,
}

pub struct Array {
    pub name: String,
    pub kind: ArrayKind,
    pub level: String,
    pub state: String,
    pub members: Vec<String>,
    pub read_bps: f64,
    pub write_bps: f64,
}

pub struct DiskMonitor {
    pub disks: Vec<Disk>,
    pub arrays: Vec<Array>,
    last: Option<Instant>,
}

impl DiskMonitor {
    pub fn new() -> Self {
        let mut m = Self { disks: Vec::new(), arrays: Vec::new(), last: None };
        m.refresh();
        m
    }

    pub fn refresh(&mut self) {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);

        let stats = read_diskstats();
        for d in &mut self.disks {
            d.seen = false;
        }

        for name in list_dir("/sys/block") {
            let Some(&raw) = stats.get(&name) else { continue };
            if skip_device(&name) {
                continue;
            }
            let idx = match self.disks.iter().position(|d| d.name == name) {
                Some(i) => i,
                None => {
                    self.disks.push(Disk::probe(&name));
                    self.disks.len() - 1
                }
            };
            self.disks[idx].update(raw, dt);
        }

        // Drop hot-unplugged devices.
        self.disks.retain(|d| d.seen);
        self.disks.sort_by(|a, b| (a.kind as u8, &a.name).cmp(&(b.kind as u8, &b.name)));

        self.refresh_arrays();
    }

    fn refresh_arrays(&mut self) {
        let mut arrays = Vec::new();

        // mdadm arrays
        for d in self.disks.iter().filter(|d| d.kind == Kind::Md) {
            let md = Path::new("/sys/block").join(&d.name).join("md");
            let level = read_trim(md.join("level")).unwrap_or_else(|| "?".into());
            let mut state = read_trim(md.join("array_state")).unwrap_or_else(|| "?".into());
            if let (Some(deg), Some(n)) = (read_u64(md.join("degraded")), read_u64(md.join("raid_disks"))) {
                if deg > 0 {
                    state = format!("DEGRADED {}/{}", n - deg.min(n), n);
                }
            }
            let action = read_trim(md.join("sync_action")).unwrap_or_default();
            if !action.is_empty() && action != "idle" {
                let pct = read_trim(md.join("sync_completed"))
                    .and_then(|s| {
                        let (a, b) = s.split_once('/')?;
                        let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
                        (b > 0.0).then(|| format!(" {:.1}%", a / b * 100.0))
                    })
                    .unwrap_or_default();
                state = format!("{state}, {action}{pct}");
            }
            arrays.push(Array {
                name: d.name.clone(),
                kind: ArrayKind::Md,
                level,
                state,
                members: physical_disks_of(&d.name),
                read_bps: d.rates.read_bps,
                write_bps: d.rates.write_bps,
            });
        }

        // btrfs multi-device filesystems (btrfs RAID has no block device of
        // its own, so throughput is the sum of the member drives).
        for uuid in list_dir("/sys/fs/btrfs") {
            let base = Path::new("/sys/fs/btrfs").join(&uuid);
            if !base.join("devices").is_dir() {
                continue;
            }
            let mut members: Vec<String> = list_dir(base.join("devices"))
                .iter()
                .flat_map(|dev| physical_disks_of(dev))
                .collect();
            members.sort();
            members.dedup();
            if members.len() < 2 {
                continue;
            }
            let label = read_trim(base.join("label")).unwrap_or_else(|| uuid[..8.min(uuid.len())].to_string());
            let level = ["data", "metadata"]
                .iter()
                .map(|kind| {
                    let profiles: Vec<String> = list_dir(base.join("allocation").join(kind))
                        .into_iter()
                        .filter(|p| base.join("allocation").join(kind).join(p).is_dir())
                        .collect();
                    profiles.join("+")
                })
                .collect::<Vec<_>>();
            let level = format!("d:{} m:{}", level[0], level[1]);
            let (mut r, mut w) = (0.0, 0.0);
            for m in &members {
                if let Some(d) = self.disks.iter().find(|d| &d.name == m) {
                    r += d.rates.read_bps;
                    w += d.rates.write_bps;
                }
            }
            arrays.push(Array {
                name: format!("btrfs:{label}"),
                kind: ArrayKind::Btrfs,
                level,
                state: "mounted".into(),
                members,
                read_bps: r,
                write_bps: w,
            });
        }

        for d in &mut self.disks {
            d.arrays = arrays
                .iter()
                .filter(|a| a.members.contains(&d.name))
                .map(|a| a.name.clone())
                .collect();
        }
        self.arrays = arrays;
    }
}

impl DiskMonitor {
    /// Read/write history of an array, summed over its member drives.
    /// Histories are aligned on the most recent sample, since a member may
    /// have appeared later than the others.
    pub fn array_history(&self, a: &Array) -> VecDeque<(f64, f64)> {
        let members: Vec<&Disk> = self.disks.iter().filter(|d| a.members.contains(&d.name)).collect();
        let len = members.iter().map(|d| d.history.len()).max().unwrap_or(0);
        let mut out: VecDeque<(f64, f64)> = std::iter::repeat((0.0, 0.0)).take(len).collect();
        for d in members {
            let skip = len - d.history.len();
            for (i, &(r, w)) in d.history.iter().enumerate() {
                out[skip + i].0 += r;
                out[skip + i].1 += w;
            }
        }
        out
    }
}

impl Disk {
    fn probe(name: &str) -> Self {
        let sys = Path::new("/sys/block").join(name);
        let kind = if name.starts_with("md") && sys.join("md").exists() {
            Kind::Md
        } else if name.starts_with("dm-") {
            Kind::Dm
        } else if sys.join("device").exists() {
            Kind::Disk
        } else {
            Kind::Virtual
        };

        let model = match kind {
            Kind::Md => read_trim(sys.join("md/level")).unwrap_or_default(),
            Kind::Dm => read_trim(sys.join("dm/name")).unwrap_or_default(),
            _ => read_trim(sys.join("device/model"))
                .or_else(|| read_trim(sys.join("device/name")))
                .unwrap_or_default(),
        };
        let media = match kind {
            Kind::Disk if name.starts_with("nvme") => "NVMe",
            Kind::Disk if name.starts_with("mmcblk") => "MMC",
            Kind::Disk => match read_u64(sys.join("queue/rotational")) {
                Some(1) => "HDD",
                _ => "SSD",
            },
            Kind::Md => "RAID",
            Kind::Dm => "DM",
            Kind::Virtual => "virt",
        };

        Self {
            name: name.to_string(),
            kind,
            model,
            media,
            size: read_u64(sys.join("size")).unwrap_or(0) * 512,
            rates: Rates::default(),
            total_read: 0,
            total_written: 0,
            history: VecDeque::with_capacity(HISTORY),
            arrays: Vec::new(),
            prev: None,
            seen: true,
        }
    }

    fn update(&mut self, raw: Raw, dt: f64) {
        self.seen = true;
        self.total_read = raw.sectors_read * 512;
        self.total_written = raw.sectors_written * 512;
        if let (Some(p), true) = (self.prev, dt > 0.0) {
            let d = |a: u64, b: u64| a.saturating_sub(b) as f64;
            let ios = d(raw.reads, p.reads) + d(raw.writes, p.writes);
            let ms = d(raw.read_ms, p.read_ms) + d(raw.write_ms, p.write_ms);
            self.rates = Rates {
                read_bps: d(raw.sectors_read, p.sectors_read) * SECTOR / dt,
                write_bps: d(raw.sectors_written, p.sectors_written) * SECTOR / dt,
                read_iops: d(raw.reads, p.reads) / dt,
                write_iops: d(raw.writes, p.writes) / dt,
                await_ms: if ios > 0.0 { ms / ios } else { 0.0 },
                util: (d(raw.io_ticks, p.io_ticks) / (dt * 1000.0) * 100.0).min(100.0),
            };
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back((self.rates.read_bps, self.rates.write_bps));
        }
        self.prev = Some(raw);
    }
}

fn skip_device(name: &str) -> bool {
    // eMMC boot / RPMB partitions and hidden NVMe multipath paths.
    name.ends_with("rpmb")
        || (name.starts_with("mmcblk") && name.contains("boot"))
        || read_u64(Path::new("/sys/block").join(name).join("hidden")) == Some(1)
}

fn read_diskstats() -> HashMap<String, Raw> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string("/proc/diskstats") else { return out };
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 14 {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        out.insert(
            f[2].to_string(),
            Raw {
                reads: n(3),
                sectors_read: n(5),
                read_ms: n(6),
                writes: n(7),
                sectors_written: n(9),
                write_ms: n(10),
                io_ticks: n(12),
            },
        );
    }
    out
}
