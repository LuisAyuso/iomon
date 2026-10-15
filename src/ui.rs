use std::collections::VecDeque;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::{App, Focus};
use crate::disk::{ArrayKind, Kind, HISTORY};

const READ: Color = Color::Cyan;
const WRITE: Color = Color::Magenta;

pub fn draw(f: &mut Frame, app: &App) {
    let disks = app.visible_disks();
    let ifaces = app.visible_ifaces();
    let arrays = &app.disks.arrays;

    let mut constraints = vec![Constraint::Length(disks.len().max(1) as u16 + 3)];
    if !arrays.is_empty() {
        constraints.push(Constraint::Length(arrays.len() as u16 + 3));
    }
    constraints.push(Constraint::Length(ifaces.len().max(1) as u16 + 3));
    constraints.push(Constraint::Min(8));
    constraints.push(Constraint::Length(1));
    let areas = Layout::vertical(constraints).split(f.area());
    let mut areas = areas.iter().copied();

    draw_disks(f, areas.next().unwrap(), app);
    if !arrays.is_empty() {
        draw_arrays(f, areas.next().unwrap(), app);
    }
    draw_net(f, areas.next().unwrap(), app);

    let charts = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(areas.next().unwrap());
    if app.chart_array {
        if let Some(a) = arrays.get(app.array_sel) {
            let title = format!(" {} history (sum of {} drives) ", a.name, a.members.len());
            let history = app.disks.array_history(a);
            draw_chart(f, charts[0], &title, &history, ("read", "write"), app, fmt_bytes_rate);
        }
    } else if let Some(d) = disks.get(app.disk_sel) {
        let title = format!(" {} history ", d.name);
        draw_chart(f, charts[0], &title, &d.history, ("read", "write"), app, fmt_bytes_rate);
    }
    if let Some(i) = ifaces.get(app.net_sel) {
        let title = format!(" {} history ", i.name);
        let fmt = if app.net_bits { fmt_bits_rate } else { fmt_bytes_rate };
        draw_chart(f, charts[1], &title, &i.history, ("rx", "tx"), app, fmt);
    }

    draw_footer(f, areas.next().unwrap(), app);
}

fn block(title: &str, focused: bool) -> Block<'static> {
    let b = Block::bordered().title(Line::from(title.to_string()).bold());
    if focused {
        b.border_style(Style::new().fg(Color::Yellow))
    } else {
        b.border_style(Style::new().fg(Color::DarkGray))
    }
}

fn header(cols: &[&'static str]) -> Row<'static> {
    Row::new(cols.iter().map(|c| Cell::from(*c))).style(Style::new().bold().fg(Color::Gray))
}

fn highlight() -> Style {
    Style::new().bg(Color::Rgb(40, 44, 60)).add_modifier(Modifier::BOLD)
}

fn draw_disks(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Disks;
    let rows = app.visible_disks().into_iter().map(|d| {
        let r = &d.rates;
        let name_style = match d.kind {
            Kind::Disk => Style::new().fg(Color::White),
            Kind::Md => Style::new().fg(Color::Yellow),
            _ => Style::new().fg(Color::DarkGray),
        };
        Row::new(vec![
            Cell::from(d.name.clone()).style(name_style),
            Cell::from(d.media),
            Cell::from(truncate(&d.model, 22)),
            Cell::from(fmt_bytes(d.size as f64)),
            rate_cell(r.read_bps, fmt_bytes_rate, READ),
            rate_cell(r.write_bps, fmt_bytes_rate, WRITE),
            Cell::from(format!("{:.0}", r.read_iops)),
            Cell::from(format!("{:.0}", r.write_iops)),
            Cell::from(format!("{:.2}", r.await_ms)),
            util_cell(r.util, d.kind),
            Cell::from(fmt_bytes(d.total_read as f64)).dark_gray(),
            Cell::from(fmt_bytes(d.total_written as f64)).dark_gray(),
            Cell::from(d.arrays.join(",")).yellow(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(22),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Min(6),
        ],
    )
    .header(header(&[
        "Device", "Type", "Model", "Size", "Read/s", "Write/s", "rIOPS", "wIOPS", "ms/io", "Util", "Total R", "Total W",
        "Array",
    ]))
    .block(block(" Drives ", focused))
    .row_highlight_style(highlight());
    let mut state = TableState::default().with_selected(focused.then_some(app.disk_sel));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_arrays(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Arrays;
    let rows = app.disks.arrays.iter().map(|a| {
        let state_style = if a.state.contains("DEGRADED") {
            Style::new().fg(Color::Red).bold()
        } else if a.state.contains("recover") || a.state.contains("resync") || a.state.contains("check") {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Green)
        };
        Row::new(vec![
            Cell::from(a.name.clone()).yellow(),
            Cell::from(match a.kind {
                ArrayKind::Md => "mdadm",
                ArrayKind::Btrfs => "btrfs",
            }),
            Cell::from(a.level.clone()),
            Cell::from(a.state.clone()).style(state_style),
            rate_cell(a.read_bps, fmt_bytes_rate, READ),
            rate_cell(a.write_bps, fmt_bytes_rate, WRITE),
            Cell::from(a.members.join(" ")),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(6),
            Constraint::Length(16),
            Constraint::Length(22),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Min(10),
        ],
    )
    .header(header(&["Array", "Kind", "Level", "State", "Read/s", "Write/s", "Member drives"]))
    .block(block(" RAID arrays ", focused))
    .row_highlight_style(highlight());
    let mut state = TableState::default().with_selected(focused.then_some(app.array_sel));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_net(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Net;
    let fmt = if app.net_bits { fmt_bits_rate } else { fmt_bytes_rate };
    let rows = app.visible_ifaces().into_iter().map(|i| {
        let up = i.state == "up" || i.state == "unknown";
        Row::new(vec![
            Cell::from(i.name.clone()),
            Cell::from(format!("{:?}", i.kind)),
            Cell::from(i.state.clone()).style(if up { Style::new().green() } else { Style::new().red() }),
            Cell::from(i.speed_mbps.map(|s| format!("{s} Mb/s")).unwrap_or_else(|| "-".into())),
            rate_cell(i.rx_bps, fmt, READ),
            rate_cell(i.tx_bps, fmt, WRITE),
            Cell::from(format!("{:.0}", i.rx_pps)),
            Cell::from(format!("{:.0}", i.tx_pps)),
            Cell::from(i.errors.to_string()).style(if i.errors > 0 { Style::new().red() } else { Style::new() }),
            Cell::from(i.drops.to_string()).style(if i.drops > 0 { Style::new().yellow() } else { Style::new() }),
            Cell::from(fmt_bytes(i.total_rx as f64)).dark_gray(),
            Cell::from(fmt_bytes(i.total_tx as f64)).dark_gray(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(9),
        ],
    )
    .header(header(&[
        "Interface", "Kind", "State", "Link", "RX/s", "TX/s", "RX pk/s", "TX pk/s", "Errors", "Drops", "Total RX",
        "Total TX",
    ]))
    .block(block(" Network ", focused))
    .row_highlight_style(highlight());
    let mut state = TableState::default().with_selected(focused.then_some(app.net_sel));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_chart(
    f: &mut Frame,
    area: Rect,
    title: &str,
    history: &VecDeque<(f64, f64)>,
    names: (&'static str, &'static str),
    app: &App,
    fmt: fn(f64) -> String,
) {
    let offset = (HISTORY - history.len()) as f64;
    let a: Vec<(f64, f64)> = history.iter().enumerate().map(|(i, v)| (offset + i as f64, v.0)).collect();
    let b: Vec<(f64, f64)> = history.iter().enumerate().map(|(i, v)| (offset + i as f64, v.1)).collect();
    let max = history.iter().map(|v| v.0.max(v.1)).fold(0.0, f64::max).max(1024.0) * 1.1;
    let cur = history.back().copied().unwrap_or_default();

    let datasets = vec![
        Dataset::default()
            .name(format!("{} {}", names.0, fmt(cur.0)))
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(READ))
            .data(&a),
        Dataset::default()
            .name(format!("{} {}", names.1, fmt(cur.1)))
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(WRITE))
            .data(&b),
    ];
    let span_secs = HISTORY as f64 * app.interval.as_secs_f64();
    let chart = Chart::new(datasets)
        .block(block(title, false))
        .x_axis(
            Axis::default()
                .bounds([0.0, HISTORY as f64])
                .labels([format!("-{span_secs:.0}s"), "now".to_string()])
                .style(Style::new().dark_gray()),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, max])
                .labels(["0".to_string(), fmt(max / 2.0), fmt(max)])
                .style(Style::new().dark_gray()),
        )
        .legend_position(Some(ratatui::widgets::LegendPosition::TopLeft))
        // Default hides the legend when it exceeds 1/4 of the area, which makes it
        // flicker as the live values in the labels change width.
        .hidden_legend_constraints((Constraint::Percentage(100), Constraint::Percentage(100)));
    f.render_widget(chart, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let key = |k: &'static str| Span::styled(k, Style::new().fg(Color::Black).bg(Color::Gray));
    let txt = |t: String| Span::raw(t);
    let mut spans = vec![
        key(" q "),
        txt(" quit  ".into()),
        key(" Tab "),
        txt(" focus  ".into()),
        key(" ↑↓ "),
        txt(" select  ".into()),
        key(" a "),
        txt(format!(" {} devices  ", if app.show_all { "physical" } else { "all" })),
        key(" b "),
        txt(format!(" net in {}  ", if app.net_bits { "bytes" } else { "bits" })),
        key(" +/- "),
        txt(format!(" interval {:.2}s  ", app.interval.as_secs_f64())),
        key(" p "),
        txt(" pause".into()),
    ];
    if app.paused {
        spans.push(Span::styled("  PAUSED", Style::new().fg(Color::Red).bold()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn rate_cell(v: f64, fmt: fn(f64) -> String, color: Color) -> Cell<'static> {
    let style = if v < 1.0 { Style::new().dark_gray() } else { Style::new().fg(color) };
    Cell::from(fmt(v)).style(style)
}

fn util_cell(util: f64, kind: Kind) -> Cell<'static> {
    // md / dm devices don't track busy time meaningfully.
    if matches!(kind, Kind::Md | Kind::Dm) {
        return Cell::from("-").dark_gray();
    }
    let color = match util {
        u if u >= 90.0 => Color::Red,
        u if u >= 60.0 => Color::Yellow,
        u if u >= 1.0 => Color::Green,
        _ => Color::DarkGray,
    };
    Cell::from(format!("{util:>3.0}%")).fg(color)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

fn scale(mut v: f64, base: f64, units: &[&str]) -> String {
    let mut i = 0;
    while v >= base && i + 1 < units.len() {
        v /= base;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", units[0])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

fn fmt_bytes(v: f64) -> String {
    scale(v, 1024.0, &["B", "KiB", "MiB", "GiB", "TiB", "PiB"])
}

fn fmt_bytes_rate(v: f64) -> String {
    scale(v, 1024.0, &["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"])
}

fn fmt_bits_rate(v: f64) -> String {
    scale(v * 8.0, 1000.0, &["b/s", "Kb/s", "Mb/s", "Gb/s", "Tb/s"])
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::App;

    #[test]
    fn renders_live_system() {
        let mut app = App::new(std::time::Duration::from_secs(1));
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            app.disks.refresh();
            app.net.refresh();
        }
        let mut term = Terminal::new(TestBackend::new(150, 36)).unwrap();
        term.draw(|f| super::draw(f, &app)).unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect();
        println!("{text}");
        assert!(text.contains("Drives") && text.contains("Network"));
    }
}
