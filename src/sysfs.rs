//! Small helpers for reading sysfs / procfs attributes.

use std::fs;
use std::path::{Path, PathBuf};

pub fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub fn list_dir(path: impl AsRef<Path>) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

pub fn class_block(name: &str) -> PathBuf {
    Path::new("/sys/class/block").join(name)
}

/// Resolve any block device (partition, dm/LUKS/LVM volume, md array, ...)
/// down to the whole physical disks that back it.
pub fn physical_disks_of(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    resolve(name, &mut out, 0);
    out.sort();
    out.dedup();
    out
}

fn resolve(name: &str, out: &mut Vec<String>, depth: u32) {
    if depth > 16 {
        return;
    }
    let base = class_block(name);
    // A partition: its parent directory in sysfs is the whole disk.
    if base.join("partition").exists() {
        if let Some(parent) = fs::canonicalize(&base)
            .ok()
            .and_then(|p| p.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()))
        {
            resolve(&parent, out, depth + 1);
        }
        return;
    }
    let slaves = list_dir(base.join("slaves"));
    if slaves.is_empty() {
        out.push(name.to_string());
    } else {
        for s in slaves {
            resolve(&s, out, depth + 1);
        }
    }
}
