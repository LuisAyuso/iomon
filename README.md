# iomon

A terminal dashboard for per-drive, RAID and network I/O bandwidth on Linux.

iomon shows live throughput, IOPS, latency and utilization for every physical
drive, the RAID arrays built on top of them, and your network interfaces, with
scrolling history charts for the selected drive/array and interface.

## Features

- **Per-drive stats** from `/proc/diskstats`: read/write throughput, IOPS,
  average latency (ms per I/O), utilization, and lifetime totals. Because it
  reads the block layer, it works with any filesystem (ext4, btrfs, xfs, zfs,
  ...) and reports what each physical drive actually does, including RAID
  overhead such as mirrored writes and parity.
- **Drive detection**: NVMe, SATA/SAS SSD vs HDD, MMC, with model and size.
- **RAID arrays**:
  - mdadm arrays with level, state, degraded member count and
    resync/recovery/check progress.
  - btrfs multi-device filesystems with their data/metadata profiles.
  - Member drives are resolved through partitions, LUKS and LVM down to the
    physical disks. Selecting an array charts the summed I/O of its members.
- **Network interfaces** from `/proc/net/dev`: RX/TX throughput and packet
  rates, link state and speed, errors and drops. Rates can be shown in bytes
  or bits.
- **History charts** covering the last 240 samples.
- Hot-plugged drives and interfaces appear and disappear automatically.

By default only physical drives, md arrays, and Ethernet/wireless/bond
interfaces are listed; press `a` to also show device-mapper volumes, loop/zram
devices, loopback and virtual interfaces.

## Requirements

- Linux (reads `/proc` and `/sys`; no root required)
- A Rust toolchain with edition 2024 support (Rust 1.85+)

## Installation

```sh
cargo install --path .
```

Or build and run from the repository:

```sh
cargo run --release
```

## Usage

```
iomon [-i SECONDS]

OPTIONS:
  -i, --interval SECONDS   sampling interval (default 1.0, range 0.1–60)
  -h, --help               show this help
```

### Keys

| Key                 | Action                                            |
| ------------------- | ------------------------------------------------- |
| `Tab` / `Shift+Tab` | Cycle focus between Drives, RAID arrays, Network  |
| `↑` `↓` / `k` `j`   | Select a row in the focused section               |
| `a`                 | Toggle between physical-only and all devices      |
| `b`                 | Toggle network rates between bytes and bits       |
| `+` / `-`           | Increase / decrease the sampling interval by 0.25s |
| `p` / `Space`       | Pause / resume sampling                           |
| `q` / `Esc` / `Ctrl+C` | Quit                                           |

The left chart follows the selected drive, or the selected array when the RAID
section was focused last; the right chart follows the selected interface.

## Notes

- Utilization is not shown for md and device-mapper devices, since the kernel
  does not track their busy time meaningfully.
- btrfs RAID has no block device of its own, so its throughput is the sum of
  its member drives.

## Development

```sh
cargo build
cargo test   # renders the UI against the live system
```
