mod app;
mod disk;
mod net;
mod sysfs;
mod ui;

use std::time::Duration;

use anyhow::{bail, Context, Result};

const USAGE: &str = "\
iomon - per-drive and network I/O bandwidth monitor

USAGE: iomon [-i SECONDS]

OPTIONS:
  -i, --interval SECONDS   sampling interval (default 1.0)
  -h, --help               show this help";

fn main() -> Result<()> {
    let mut interval = Duration::from_secs(1);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-i" | "--interval" => {
                let v: f64 = args
                    .next()
                    .context("missing value for --interval")?
                    .parse()
                    .context("invalid --interval")?;
                if !(0.1..=60.0).contains(&v) {
                    bail!("--interval must be between 0.1 and 60 seconds");
                }
                interval = Duration::from_secs_f64(v);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument: {other}\n\n{USAGE}"),
        }
    }

    if !std::path::Path::new("/proc/diskstats").exists() {
        bail!("/proc/diskstats not found - iomon requires Linux");
    }

    let mut terminal = ratatui::init();
    let res = app::App::new(interval).run(&mut terminal);
    ratatui::restore();
    res
}
