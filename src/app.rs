use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::disk::{self, Disk, DiskMonitor};
use crate::net::{self, Iface, NetMonitor};
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Disks,
    Arrays,
    Net,
}

pub struct App {
    pub interval: Duration,
    pub disks: DiskMonitor,
    pub net: NetMonitor,
    pub focus: Focus,
    pub disk_sel: usize,
    pub array_sel: usize,
    pub net_sel: usize,
    /// Whether the left chart shows the selected array (aggregated over its
    /// member drives) rather than the selected drive. Follows whichever of
    /// the two storage sections was focused last.
    pub chart_array: bool,
    pub show_all: bool,
    pub net_bits: bool,
    pub paused: bool,
    quit: bool,
}

impl App {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            disks: DiskMonitor::new(),
            net: NetMonitor::new(),
            focus: Focus::Disks,
            disk_sel: 0,
            array_sel: 0,
            net_sel: 0,
            chart_array: false,
            show_all: false,
            net_bits: false,
            paused: false,
            quit: false,
        }
    }

    pub fn visible_disks(&self) -> Vec<&Disk> {
        self.disks
            .disks
            .iter()
            .filter(|d| self.show_all || matches!(d.kind, disk::Kind::Disk | disk::Kind::Md))
            .collect()
    }

    pub fn visible_ifaces(&self) -> Vec<&Iface> {
        self.net
            .ifaces
            .iter()
            .filter(|i| {
                self.show_all || matches!(i.kind, net::Kind::Ethernet | net::Kind::Wireless | net::Kind::Bond)
            })
            .collect()
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut next_tick = Instant::now() + self.interval;
        while !self.quit {
            self.clamp_selection();
            terminal.draw(|f| ui::draw(f, &self))?;

            let timeout = next_tick.saturating_duration_since(Instant::now());
            if event::poll(timeout)? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.on_key(k.code, k.modifiers);
                    }
                }
            }
            if Instant::now() >= next_tick {
                if !self.paused {
                    self.disks.refresh();
                    self.net.refresh();
                }
                next_tick = Instant::now() + self.interval;
            }
        }
        Ok(())
    }

    fn clamp_selection(&mut self) {
        let nd = self.visible_disks().len();
        let ni = self.visible_ifaces().len();
        let na = self.disks.arrays.len();
        self.disk_sel = self.disk_sel.min(nd.saturating_sub(1));
        self.array_sel = self.array_sel.min(na.saturating_sub(1));
        if na == 0 {
            self.chart_array = false;
            if self.focus == Focus::Arrays {
                self.focus = Focus::Disks;
            }
        }
        self.net_sel = self.net_sel.min(ni.saturating_sub(1));
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let sel = match self.focus {
            Focus::Disks => &mut self.disk_sel,
            Focus::Arrays => &mut self.array_sel,
            Focus::Net => &mut self.net_sel,
        };
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Tab | KeyCode::BackTab => {
                let mut order = vec![Focus::Disks];
                if !self.disks.arrays.is_empty() {
                    order.push(Focus::Arrays);
                }
                order.push(Focus::Net);
                let i = order.iter().position(|&f| f == self.focus).unwrap_or(0);
                let n = order.len();
                self.focus = if code == KeyCode::Tab { order[(i + 1) % n] } else { order[(i + n - 1) % n] };
                match self.focus {
                    Focus::Disks => self.chart_array = false,
                    Focus::Arrays => self.chart_array = true,
                    Focus::Net => {}
                }
            }
            KeyCode::Up | KeyCode::Char('k') => *sel = sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *sel += 1,
            KeyCode::Char('a') => self.show_all = !self.show_all,
            KeyCode::Char('b') => self.net_bits = !self.net_bits,
            KeyCode::Char('p') | KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval = (self.interval + Duration::from_millis(250)).min(Duration::from_secs(10))
            }
            KeyCode::Char('-') => {
                self.interval = self
                    .interval
                    .saturating_sub(Duration::from_millis(250))
                    .max(Duration::from_millis(250))
            }
            _ => {}
        }
    }
}
