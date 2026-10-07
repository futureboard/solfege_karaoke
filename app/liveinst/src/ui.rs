//! Rendering. Layout:
//!   header (audio/MIDI status, CPU, master meter)
//!   rack table | slot parameters
//!   MIDI file player
//!   keyboard (active notes of the selected slot)
//!   MIDI monitor | log
//!   key hints

use std::sync::atomic::Ordering;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};

use crate::app::{
    App, Focus, MixRow, Popup, PortItem, STRIP_FIELDS, channels_short, channels_text, format_time, fx_field_text, output_name,
    pan_text, strip_field_text,
};
use crate::engine::load_peak;
use crate::engine::PlayState;
use crate::instrument::{Kind, note_name};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const METER_W: usize = 12;

fn block(title: &str, active: bool) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if active { ACCENT } else { DIM }))
        .title(Span::styled(format!(" {title} "), Style::new().fg(if active { ACCENT } else { Color::Gray }).add_modifier(Modifier::BOLD)))
}

fn to_db(x: f32) -> f32 {
    if x <= 1.0e-6 { -120.0 } else { 20.0 * x.log10() }
}

fn meter_spans(level: f32, width: usize) -> Vec<Span<'static>> {
    let db = to_db(level);
    let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let filled = (frac * width as f32).round() as usize;
    (0..width)
        .map(|i| {
            let pos_db = -60.0 + 60.0 * (i as f32 + 0.5) / width as f32;
            let color = if pos_db > -3.0 { Color::Red } else if pos_db > -12.0 { Color::Yellow } else { Color::Green };
            if i < filled {
                Span::styled("█", Style::new().fg(color))
            } else {
                Span::styled("·", Style::new().fg(DIM))
            }
        })
        .collect()
}

fn kind_color(k: Kind) -> Color {
    match k {
        Kind::Wav => Color::LightGreen,
        Kind::Sfz => Color::LightMagenta,
        Kind::Sf2 => Color::LightBlue,
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let [header, main, player, keys, bottom, footer] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(8),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(9),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    let [rack, params] = Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)]).areas(main);
    if app.focus == Focus::Mixer && app.sel < app.slots.len() {
        draw_mixer(f, app, main);
    } else {
        if app.focus == Focus::Channels && app.sel < app.slots.len() {
            draw_channels(f, app, rack);
        } else {
            draw_rack(f, app, rack);
        }
        draw_params(f, app, params);
    }
    draw_player(f, app, player);
    draw_keyboard(f, app, keys);
    let [mon, log] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(bottom);
    draw_monitor(f, app, mon);
    draw_log(f, app, log);
    draw_footer(f, app, footer);

    match &app.popup {
        Popup::None => {}
        Popup::Help => draw_help(f),
        Popup::Browser(b, _) => draw_browser(f, b),
        Popup::Ports { items, sel } => draw_ports(f, app, items, *sel),
        Popup::Devices { items, sel } => draw_devices(f, app, items, *sel),
        Popup::Presets { sel, filter, channel } => draw_presets(f, app, *sel, filter, *channel),
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let b = block("simpletui · instrument rack", false);
    let inner = b.inner(area);
    f.render_widget(b, area);

    let audio = match (&app.audio, &app.audio_error) {
        (Some(a), _) => Line::from(vec![
            Span::styled("Audio ", Style::new().fg(DIM)),
            Span::styled(a.host.clone(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" · {} · {} Hz · {} · {}ch · buf {}", a.device, a.sample_rate, a.format, a.channels, a.buffer)),
        ]),
        (None, Some(e)) => Line::from(Span::styled(e.clone(), Style::new().fg(Color::Red))),
        (None, None) => Line::from(Span::styled("audio not started", Style::new().fg(Color::Red))),
    };
    let ins = app.midi.connected_inputs();
    let midi_in = if ins.is_empty() { "none".to_string() } else { ins.join(", ") };
    let midi_out = app.midi.output_name().unwrap_or_else(|| "none".into());
    let thru = app.midi.thru.load(Ordering::Relaxed);
    let mut midi_spans = Vec::new();
    if let Some(url) = &app.web_url {
        midi_spans.push(Span::styled("Web ", Style::new().fg(DIM)));
        midi_spans.push(Span::styled(format!("{url}  "), Style::new().fg(Color::LightBlue)));
    }
    let midi = Line::from([midi_spans, vec![
        Span::styled("MIDI in ", Style::new().fg(DIM)),
        Span::raw(midi_in),
        Span::styled("  out ", Style::new().fg(DIM)),
        Span::raw(midi_out),
        Span::styled(if thru { "  [thru]" } else { "" }, Style::new().fg(Color::Yellow)),
    ]].concat());

    let [left, right] = Layout::horizontal([Constraint::Min(20), Constraint::Length(44)]).areas(inner);
    f.render_widget(Paragraph::new(vec![audio, midi]), left);

    let errors = app.shared.errors.load(Ordering::Relaxed);
    let cpu_color = if app.cpu > 0.8 { Color::Red } else if app.cpu > 0.5 { Color::Yellow } else { Color::Green };
    let mut l1 = vec![
        Span::styled("CPU ", Style::new().fg(DIM)),
        Span::styled(format!("{:>3.0}%", app.cpu * 100.0), Style::new().fg(cpu_color)),
        Span::styled("  Voices ", Style::new().fg(DIM)),
        Span::raw(format!("{:<3}", app.voices)),
        Span::styled("  Master ", Style::new().fg(DIM)),
        Span::raw(format!("{:+.0}dB", app.master_db)),
    ];
    if errors > 0 {
        l1.push(Span::styled(format!(" err {errors}"), Style::new().fg(Color::Red)));
    }
    let mut l2 = vec![Span::styled("L ", Style::new().fg(DIM))];
    l2.extend(meter_spans(app.master_meter[0], 18));
    l2.push(Span::styled(" R ", Style::new().fg(DIM)));
    l2.extend(meter_spans(app.master_meter[1], 18));
    f.render_widget(Paragraph::new(vec![Line::from(l1), Line::from(l2)]), right);
}

fn draw_rack(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Rack && !app.play_mode;
    let title = format!("Rack {}/{}", app.slots.len(), crate::engine::MAX_SLOTS);
    let b = block(&title, active || app.play_mode);
    if app.slots.is_empty() {
        let mut lines = vec![
            Line::raw(""),
            Line::from(vec![Span::raw("  Rack is empty. Press "), Span::styled("a", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)), Span::raw(" to add an instrument (.wav .sfz .sf2).")]),
            Line::raw("  Or pass files on the command line:  simpletui piano.sf2 drums.sfz kick.wav"),
        ];
        for p in &app.loading {
            lines.push(Line::styled(format!("  loading {} ...", p.display()), Style::new().fg(Color::Yellow)));
        }
        f.render_widget(Paragraph::new(lines).block(b), area);
        return;
    }

    let any_solo = app.slots.iter().any(|s| s.params.solo);
    let rows: Vec<Row> = app
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let p = &s.params;
            let silent = p.mute || (any_solo && !p.solo);
            let preset = if s.inst.kind == Kind::Sf2 {
                let pr = &s.inst.presets[s.preset];
                let extra = app.custom_channels(i);
                let multi = if p.multitimbral() && extra > 0 { format!(" +{extra}ch") } else { String::new() };
                format!("{:03}:{:03} {}{multi}", pr.bank, pr.program, pr.name)
            } else {
                let n = s.inst.zone_count();
                format!("{n} zone{}", if n == 1 { "" } else { "s" })
            };
            let keys = if p.key_lo == 0 && p.key_hi == 127 {
                "all".to_string()
            } else {
                format!("{}-{}", note_name(p.key_lo), note_name(p.key_hi))
            };
            let ms = Line::from(vec![
                Span::styled("M", Style::new().fg(if p.mute { Color::Black } else { DIM }).bg(if p.mute { Color::Red } else { Color::Reset })),
                Span::raw(" "),
                Span::styled("S", Style::new().fg(if p.solo { Color::Black } else { DIM }).bg(if p.solo { Color::Yellow } else { Color::Reset })),
            ]);
            let voices = app.shared.slots[i].voices.load(Ordering::Relaxed);
            let level = s.meter[0].max(s.meter[1]);
            let name_style = if silent { Style::new().fg(DIM) } else { Style::new().fg(Color::White).add_modifier(Modifier::BOLD) };
            Row::new(vec![
                Cell::from(format!("{:>2}", i + 1)),
                Cell::from(Span::styled(s.inst.name.clone(), name_style)),
                Cell::from(Span::styled(s.inst.kind.label(), Style::new().fg(kind_color(s.inst.kind)))),
                Cell::from(preset),
                Cell::from(channels_short(p.channels, 9)),
                Cell::from(keys),
                Cell::from(format!("{:+.1}", s.volume_db)),
                Cell::from(pan_text(p.pan)),
                Cell::from(ms),
                Cell::from(Line::from(meter_spans(level, METER_W))),
                Cell::from(format!("{voices:>3}")),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(2),
        Constraint::Fill(2),
        Constraint::Length(4),
        Constraint::Fill(3),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Length(METER_W as u16),
        Constraint::Length(3),
    ];
    let header = Row::new(["#", "Name", "Type", "Preset / Map", "Ch", "Keys", "Vol", "Pan", "M S", "Level", "Vc"])
        .style(Style::new().fg(DIM).add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .block(b)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)))
        .highlight_symbol("▶");
    let mut state = TableState::default().with_selected(Some(app.sel));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_params(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Params && !app.play_mode;
    let Some(s) = app.slots.get(app.sel) else {
        f.render_widget(Paragraph::new("").block(block("Slot", false)), area);
        return;
    };
    let title = format!("Slot {} · {}", app.sel + 1, s.inst.kind.label());
    let b = block(&title, active);
    let inner = b.inner(area);
    f.render_widget(b, area);

    let params = app.params_for(app.sel);
    let [list_area, info_area] = Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).areas(inner);
    let items: Vec<ListItem> = params
        .iter()
        .map(|&p| {
            let (label, value) = app.param_text(app.sel, p);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{label:<13}"), Style::new().fg(Color::Gray)),
                Span::styled(value, Style::new().fg(Color::White)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(if active { Style::new().bg(Color::Rgb(30, 50, 70)).fg(ACCENT) } else { Style::new() })
        .highlight_symbol(if active { "▶ " } else { "  " });
    let mut state = ListState::default().with_selected(Some(app.param_sel.min(params.len().saturating_sub(1))));
    f.render_stateful_widget(list, list_area, &mut state);

    let mut info = vec![
        Line::styled(s.inst.path.display().to_string(), Style::new().fg(DIM)),
        Line::styled(
            format!(
                "{} preset(s) · {} zone(s) · {:.1} MB",
                s.inst.presets.len(),
                s.inst.zone_count(),
                s.inst.sample_bytes as f64 / 1_048_576.0
            ),
            Style::new().fg(DIM),
        ),
    ];
    if !s.inst.warnings.is_empty() {
        info.push(Line::styled(format!("{} warning(s): {}", s.inst.warnings.len(), s.inst.warnings[0]), Style::new().fg(Color::Yellow)));
    }
    f.render_widget(Paragraph::new(info).wrap(Wrap { trim: true }), info_area);
}

fn draw_channels(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.slots[app.sel];
    let omni = s.params.multitimbral();
    let mode = match s.params.single() {
        _ if s.params.channels == crate::engine::OMNI => "Omni: 16-part multitimbral".to_string(),
        Some(c) => format!("receives ch {}", c + 1),
        None if s.params.channels == 0 => "receives nothing".to_string(),
        None => format!("receives {} (multitimbral)", channels_text(s.params.channels)),
    };
    let title = format!("Channels · slot {} · {} · {mode}", app.sel + 1, s.inst.name);
    let b = block(&title, true);
    const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let rows: Vec<Row> = (0..16)
        .map(|c| {
            let info = app.channel_info(app.sel, c);
            let listening = s.params.receives(c as u8);
            let preset = s.inst.presets.get(info.preset);
            let muted = app.player.mutes & (1 << c) != 0;
            let dim = Style::new().fg(DIM);
            let base = if !listening {
                dim
            } else if info.voices > 0 {
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::Gray)
            };
            let source = if !listening {
                Span::styled("—", dim)
            } else if info.locked {
                Span::styled("locked", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else if info.fallback {
                Span::styled("fallback", Style::new().fg(Color::Yellow))
            } else if info.explicit {
                Span::styled("program", Style::new().fg(Color::LightGreen))
            } else if info.drum && omni && preset.is_some_and(|p| p.bank == 128) {
                Span::styled("drum part", Style::new().fg(Color::LightMagenta))
            } else {
                Span::styled("default", dim)
            };
            let name = preset
                .map(|p| format!("{:03}:{:03} {}", p.bank, p.program, p.name))
                .unwrap_or_else(|| "—".into());
            let pan = match info.pan as i32 - 64 {
                0 => "C".to_string(),
                v if v < 0 => format!("L{}", -v),
                v => format!("R{v}"),
            };
            let level = ((info.voices as f32 / 4.0).min(1.0) * 8.0).round() as usize;
            Row::new(vec![
                Cell::from(if listening {
                    Span::styled("●", Style::new().fg(Color::LightGreen))
                } else {
                    Span::styled("○", Style::new().fg(DIM))
                }),
                Cell::from(Span::styled(
                    if muted { format!("{:>2}×", c + 1) } else { format!("{:>2}", c + 1) },
                    if muted { Style::new().fg(Color::Red) } else { base },
                )),
                Cell::from(format!("{:03}:{:03}", info.bank_msb, info.bank_lsb)),
                Cell::from(info.program.map(|p| format!("{p:03}")).unwrap_or_else(|| "---".into())),
                Cell::from(Span::styled(name, base)),
                Cell::from(source),
                Cell::from(format!("{:>3}", info.volume)),
                Cell::from(pan),
                Cell::from(format!("{:>3}", info.expression)),
                Cell::from(format!("{:+5}", info.bend)),
                Cell::from(if info.sustain { "▼" } else { " " }),
                Cell::from(Line::from(vec![
                    Span::styled(BARS[level], Style::new().fg(if c == 9 { Color::LightMagenta } else { Color::LightGreen })),
                    Span::raw(format!("{:>3}", info.voices)),
                ])),
            ])
            .style(base)
        })
        .collect();
    let widths = [
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(7),
        Constraint::Length(4),
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(4),
    ];
    let header = Row::new(["Rx", "Ch", "Bank", "Prg", "Preset", "Source", "Vol", "Pan", "Expr", "Bend", "P", "Vc"])
        .style(Style::new().fg(DIM).add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .block(b)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)))
        .highlight_symbol("▶");
    let mut state = TableState::default().with_selected(Some(app.chan_sel));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_mixer(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.slots[app.sel];
    let pairs = app.shared.out_pairs.load(Ordering::Relaxed).max(1);
    let title = format!(
        "Mixer · slot {} · {} · ch10 groups {} (g) · device outs {}",
        app.sel + 1,
        s.inst.name,
        if s.mixer.groups.enabled { "on" } else { "off" },
        (1..=pairs).map(|p| output_name(p as u8 - 1)).collect::<Vec<_>>().join(" ")
    );
    let rows_model = app.mixer_rows();
    let meter = &app.shared.slots[app.sel];
    let live = meter.strips_live.load(Ordering::Relaxed);
    let active = !app.play_mode;
    let sel_style = Style::new().bg(Color::Rgb(30, 50, 70));
    let cursor = Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD);
    let rows: Vec<Row> = rows_model
        .iter()
        .enumerate()
        .map(|(ri, row)| {
            let selected = active && ri == app.mix_row;
            let field_cell = |fi: usize, text: String| {
                let st = if selected && fi == app.mix_field { cursor } else { Style::new() };
                Cell::from(Span::styled(text, st))
            };
            match *row {
                MixRow::Strip(k) => {
                    let p = &s.mixer.strips[k];
                    let level = load_peak(&meter.strip_l[k]).max(load_peak(&meter.strip_r[k]));
                    let is_live = live & (1 << k) != 0;
                    let name_style = if p.mute {
                        Style::new().fg(DIM)
                    } else if is_live {
                        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().fg(Color::Gray)
                    };
                    let name_color = if k >= crate::engine::mixer::DRUM_STRIP_BASE { Color::LightMagenta } else { name_style.fg.unwrap_or(Color::Gray) };
                    let ms = Line::from(vec![
                        Span::styled("M", Style::new().fg(if p.mute { Color::Black } else { DIM }).bg(if p.mute { Color::Red } else { Color::Reset })),
                        Span::raw(" "),
                        Span::styled("S", Style::new().fg(if p.solo { Color::Black } else { DIM }).bg(if p.solo { Color::Yellow } else { Color::Reset })),
                    ]);
                    let mut cells = vec![
                        Cell::from(Span::styled(app.strip_name(app.sel, k), name_style.fg(name_color))),
                        Cell::from(Line::from(meter_spans(level, 10))),
                        Cell::from(ms),
                    ];
                    cells.extend((0..STRIP_FIELDS.len()).map(|fi| field_cell(fi, strip_field_text(p, fi))));
                    Row::new(cells).style(if selected { sel_style } else { Style::new() })
                }
                MixRow::Reverb | MixRow::Chorus => {
                    let (name, level) = if *row == MixRow::Reverb {
                        ("FX Reverb  ret/room/damp/width", load_peak(&app.shared.fx_peak[0]))
                    } else {
                        ("FX Chorus  ret/rate/depth/delay", load_peak(&app.shared.fx_peak[1]))
                    };
                    let mut cells = vec![
                        Cell::from(Span::styled(name, Style::new().fg(Color::LightCyan))),
                        Cell::from(Line::from(meter_spans(level, 10))),
                        Cell::from(""),
                    ];
                    cells.extend((0..4).map(|fi| field_cell(fi, fx_field_text(&app.fx, *row, fi))));
                    Row::new(cells).style(if selected { sel_style } else { Style::new() })
                }
                MixRow::Master => {
                    let level = app.master_meter[0].max(app.master_meter[1]);
                    Row::new(vec![
                        Cell::from(Span::styled("MASTER", Style::new().fg(Color::White).add_modifier(Modifier::BOLD))),
                        Cell::from(Line::from(meter_spans(level, 10))),
                        Cell::from(""),
                        field_cell(0, format!("{:+.1}", app.master_db)),
                    ])
                    .style(if selected { sel_style } else { Style::new() })
                }
            }
        })
        .collect();
    let mut widths = vec![Constraint::Fill(1), Constraint::Length(10), Constraint::Length(3)];
    widths.extend([6, 4, 5, 5, 5, 5, 5, 5, 5, 5].map(Constraint::Length));
    let mut header = vec!["Strip", "Level", "M S"];
    header.extend(STRIP_FIELDS);
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(Style::new().fg(DIM).add_modifier(Modifier::BOLD)))
        .block(block(&title, true))
        .column_spacing(1)
        .highlight_symbol("▶");
    let mut state = TableState::default().with_selected(Some(app.mix_row));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_player(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.player;
    let active = app.focus == Focus::Player && !app.play_mode;
    let b = block("MIDI Player", active);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let Some(song) = &p.song else {
        let line = Line::from(vec![
            Span::styled("  no song · press ", Style::new().fg(DIM)),
            Span::styled("f", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" to open a .mid file (an SF2 slot on Omni plays General MIDI)", Style::new().fg(DIM)),
        ]);
        f.render_widget(Paragraph::new(vec![Line::raw(""), line]), inner);
        return;
    };

    let (icon, color) = match p.state {
        PlayState::Playing => ("▶ PLAY ", Color::LightGreen),
        PlayState::Paused => ("❚❚ PAUSE", Color::Yellow),
        _ => ("■ STOP ", Color::Gray),
    };
    let flag = |on: bool, label: &str| {
        Span::styled(format!("  {label}"), Style::new().fg(if on { Color::Yellow } else { DIM }))
    };
    let forward = app.shared.forward_player.load(Ordering::Relaxed);
    let l1 = Line::from(vec![
        Span::styled(format!(" {icon} "), Style::new().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {}", song.name), Style::new().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(format!("   {} / {}", format_time(p.time), format_time(song.duration))),
        Span::styled(format!("   {:.0} BPM · speed {:.0}%", song.bpm * p.speed, p.speed * 100.0), Style::new().fg(Color::Gray)),
        flag(p.looping, "loop"),
        flag(forward, "→ MIDI out"),
        Span::styled(format!("   SMF {} · {} tracks · {} events", song.format, song.tracks, song.events.len()), Style::new().fg(DIM)),
    ]);

    let w = inner.width as usize;
    let frac = if song.duration > 0.0 { (p.time / song.duration).clamp(0.0, 1.0) } else { 0.0 };
    let filled = (frac * w as f64).round() as usize;
    let l2 = Line::from(vec![
        Span::styled("━".repeat(filled), Style::new().fg(ACCENT)),
        Span::styled("─".repeat(w.saturating_sub(filled)), Style::new().fg(DIM)),
    ]);

    const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let cell = (w / 16).clamp(4, 8);
    let mut l3 = Vec::new();
    for c in 0..16 {
        let muted = p.mutes & (1 << c) != 0;
        let used = song.channels_used & (1 << c) != 0;
        let level = (p.levels[c] * 8.0).round().clamp(0.0, 8.0) as usize;
        let num_style = if muted {
            Style::new().fg(Color::Red)
        } else if used {
            Style::new().fg(Color::White)
        } else {
            Style::new().fg(DIM)
        };
        let num_style = if active && c == p.channel { num_style.bg(Color::Rgb(30, 50, 70)).add_modifier(Modifier::BOLD) } else { num_style };
        let label = if muted { format!("{:>2}×", c + 1) } else { format!("{:>2} ", c + 1) };
        l3.push(Span::styled(label, num_style));
        let bar_color = if c == 9 { Color::LightMagenta } else { Color::LightGreen };
        l3.push(Span::styled(BARS[if muted { 0 } else { level }], Style::new().fg(bar_color)));
        l3.push(Span::raw(" ".repeat(cell.saturating_sub(4))));
    }
    f.render_widget(Paragraph::new(vec![l1, l2, Line::from(l3)]), inner);
}

fn is_black(n: u8) -> bool {
    matches!(n % 12, 1 | 3 | 6 | 8 | 10)
}

fn draw_keyboard(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.play_mode {
        format!(
            "Keyboard · PLAY · Z=C{} Q=C{} · vel {} · {}",
            app.octave,
            app.octave + 1,
            app.velocity,
            if app.release_events { "key-up" } else { "auto-release" }
        )
    } else {
        "Keyboard · Space to play".to_string()
    };
    let b = block(&title, app.play_mode);
    let inner = b.inner(area);
    f.render_widget(b, area);
    if app.slots.is_empty() || inner.height < 2 {
        return;
    }

    let notes = app.slot_notes(app.sel);
    let on = |n: u8| -> bool {
        let bits = if n < 64 { (notes[0] | app.kbd_notes[0]) >> n } else { (notes[1] | app.kbd_notes[1]) >> (n - 64) };
        bits & 1 == 1
    };
    let p = &app.slots[app.sel].params;
    let in_range = |n: u8| n >= p.key_lo && n <= p.key_hi;

    // Two columns per white key; centre the view on the play octave.
    let whites_fit = (inner.width as usize / 2).max(7);
    let total_whites = (0u8..128).filter(|&n| !is_black(n)).count();
    let play_c = ((app.octave + 1) * 12).clamp(0, 127) as u8;
    let play_white = (0..play_c).filter(|&n| !is_black(n)).count();
    let first_white = play_white.saturating_sub(whites_fit / 3).min(total_whites.saturating_sub(whites_fit));
    let whites: Vec<u8> = (0u8..128).filter(|&n| !is_black(n)).skip(first_white).take(whites_fit).collect();

    let active = Color::LightGreen;
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    let mut labels = Vec::new();
    for &w in &whites {
        let wbg = if on(w) { active } else if in_range(w) { Color::Gray } else { Color::DarkGray };
        top.push(Span::styled(" ", Style::new().bg(wbg)));
        let b = w + 1;
        if b < 128 && is_black(b) {
            let bbg = if on(b) { active } else if in_range(b) { Color::Black } else { Color::Rgb(40, 40, 40) };
            top.push(Span::styled("▐", Style::new().fg(bbg).bg(wbg)));
        } else {
            top.push(Span::styled("▕", Style::new().fg(Color::Black).bg(wbg)));
        }
        bottom.push(Span::styled(" ▕", Style::new().fg(Color::Black).bg(wbg)));
        let label = if w % 12 == 0 { format!("{:<2}", note_name(w)) } else { "  ".into() };
        labels.push(Span::styled(label, Style::new().fg(if w == play_c && app.play_mode { ACCENT } else { DIM })));
    }
    let mut lines = vec![Line::from(top), Line::from(bottom)];
    if inner.height >= 3 {
        lines.push(Line::from(labels));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_monitor(f: &mut Frame, app: &App, area: Rect) {
    let b = block("MIDI monitor", false);
    let h = b.inner(area).height as usize;
    let lines: Vec<Line> = app
        .midi_log
        .iter()
        .rev()
        .take(h)
        .rev()
        .map(|l| Line::styled(l.clone(), Style::new().fg(Color::Gray)))
        .collect();
    f.render_widget(Paragraph::new(lines).block(b), area);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let b = block("Log", false);
    let h = b.inner(area).height as usize;
    let lines: Vec<Line> = app
        .log
        .iter()
        .rev()
        .take(h)
        .rev()
        .map(|l| Line::styled(l.text.clone(), Style::new().fg(if l.error { Color::LightRed } else { Color::Gray })))
        .collect();
    f.render_widget(Paragraph::new(lines).block(b), area);
}

fn hint(k: &str, d: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {k} "), Style::new().fg(Color::Black).bg(ACCENT)),
        Span::styled(format!(" {d}  "), Style::new().fg(Color::Gray)),
    ]
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let pairs: &[(&str, &str)] = if app.play_mode {
        &[("Z..M Q..P", "notes"), ("- =", "octave"), ("[ ]", "velocity"), ("↑↓", "slot"), ("Bksp", "panic"), ("Esc", "stop")]
    } else if app.focus == Focus::Params {
        &[("↑↓", "param"), ("←→", "adjust"), ("Shift", "coarse"), ("Tab", "player"), ("Esc", "rack"), ("?", "help")]
    } else if app.focus == Focus::Mixer {
        &[
            ("↑↓", "strip"),
            ("←→", "field"),
            ("+ -", "adjust"),
            ("PgUp/Dn", "coarse"),
            ("0", "reset"),
            ("m/s", "mute/solo"),
            ("g", "ch10 groups"),
            ("Tab", "player"),
            ("Esc", "rack"),
        ]
    } else if app.focus == Focus::Channels {
        &[
            ("↑↓", "channel"),
            ("Space", "receive on/off"),
            ("A/I/O", "all/invert/only"),
            ("←→", "preset"),
            ("Enter", "pick preset"),
            ("d/D", "unpin one/all"),
            ("m/s/u", "mute/solo/unmute"),
            ("Tab", "player"),
            ("Esc", "rack"),
        ]
    } else if app.focus == Focus::Player {
        &[
            ("Enter", "play/pause"),
            ("S", "stop"),
            ("↑↓", "seek ±5s"),
            ("←→", "channel"),
            ("m/s/u", "mute/solo/unmute"),
            ("( )", "speed"),
            ("l", "loop"),
            ("w", "→MIDI out"),
            ("f", "open"),
            ("Tab", "rack"),
        ]
    } else {
        &[
            ("a", "add"),
            ("r", "replace"),
            ("x", "remove"),
            ("Tab", "params"),
            ("Space", "play"),
            ("m/s", "mute/solo"),
            ("c", "channel"),
            ("p , .", "preset"),
            ("f Enter", "song"),
            ("M", "mixer"),
            ("i", "MIDI"),
            ("o", "audio"),
            ("?", "help"),
            ("q", "quit"),
        ]
    };
    let spans: Vec<Span> = pairs.iter().flat_map(|(k, d)| hint(k, d)).collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn popup_area(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_help(f: &mut Frame) {
    let area = popup_area(f.area(), 80, 56);
    f.render_widget(Clear, area);
    let rows: &[(&str, &str)] = &[
        ("Rack", ""),
        ("↑ ↓", "select slot"),
        ("a / Insert", "add instrument (WAV, SFZ, SF2)"),
        ("r", "replace instrument in selected slot"),
        ("x / Delete", "remove selected slot"),
        ("← →  (Shift)", "volume ±0.5 dB (±6 dB)"),
        ("[ ]", "pan"),
        ("m  s", "mute / solo"),
        ("c  C", "MIDI channels: Omni, 1-16, no-drums (custom: v)"),
        ("p  ,  .", "preset list / previous / next (SF2)"),
        ("Tab", "edit slot parameters (key split, tune, WAV envelope...)"),
        ("v", "SoundFont channel map: bank/program per MIDI channel"),
        ("+ -", "master volume"),
        ("Backspace", "panic (all sound off, also sent to MIDI out)"),
        ("", ""),
        ("Play mode", ""),
        ("Space / k", "toggle computer keyboard piano"),
        ("Z S X D C ...", "lower octave, Q 2 W 3 E ... upper octave"),
        ("- =   [ ]", "octave down/up, velocity down/up"),
        ("", ""),
        ("SoundFont channels", ""),
        ("Omni", "one SF2 plays all 16 channels, each with its own program"),
        ("Bank select", "CC0/CC32 + Program: GM, GS (MSB), XG (MSB 127 = drums)"),
        ("v  ←→  Enter", "open map, step preset, pick preset for a channel"),
        ("d  D", "unpin channel / unpin all (pins survive resets)"),
        ("Space  x", "toggle whether this slot receives the channel (Rx)"),
        ("A  I  O", "receive all / invert / only this channel"),
        ("c  C", "cycle Omni, 1..16, 1-9,11-16 (drums split off)"),
        ("", ""),
        ("Mixer  (M)", "one strip per MIDI channel, ch10 split by note group"),
        ("↑↓ ←→  + -", "select strip / field, adjust (PgUp/PgDn coarse)"),
        ("m s 0 g", "strip mute / solo / reset field / ch10 groups on-off"),
        ("Out", "Main or device pair 3/4, 5/6... (multi-out)"),
        ("", ""),
        ("MIDI player", ""),
        ("f", "open MIDI file (.mid .kar .rmi)"),
        ("Enter / F5", "play / pause        S / F6  stop"),
        ("{ }  F7 F8", "seek -5 s / +5 s"),
        ("( )", "speed -5 % / +5 %          l  loop"),
        ("w", "also send player events to MIDI out"),
        ("Tab -> Player", "←→ channel, m mute, s solo, u unmute all, ↑↓ seek"),
        ("", ""),
        ("I/O", ""),
        ("i", "MIDI ports: toggle inputs, choose output, thru"),
        ("t", "toggle MIDI thru (inputs -> output)"),
        ("o", "choose audio output device"),
        ("", ""),
        ("Browser", "type to filter, Enter open, Backspace/← up, Esc cancel"),
        ("q / Ctrl+C", "quit"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            if d.is_empty() {
                Line::styled(k.to_string(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Line::from(vec![Span::styled(format!("  {k:<16}"), Style::new().fg(Color::White)), Span::styled(d.to_string(), Style::new().fg(Color::Gray))])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block("Help", true)), area);
}

fn draw_browser(f: &mut Frame, b: &crate::browser::Browser) {
    let area = popup_area(f.area(), 90, 30);
    f.render_widget(Clear, area);
    let what = match b.kind {
        crate::browser::BrowseKind::Instrument => "Open instrument",
        crate::browser::BrowseKind::Song => "Open MIDI file",
    };
    let title = format!("{what} · {}", b.title());
    let blk = block(&title, true);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    let [filter_area, list_area] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    let filter = match &b.error {
        Some(e) => Line::styled(e.clone(), Style::new().fg(Color::Red)),
        None if b.filter.is_empty() => Line::styled(format!("type to filter · {}", b.kind.hint()), Style::new().fg(DIM)),
        None => Line::from(vec![Span::styled("filter: ", Style::new().fg(DIM)), Span::styled(b.filter.clone(), Style::new().fg(Color::Yellow))]),
    };
    f.render_widget(Paragraph::new(filter), filter_area);
    let items: Vec<ListItem> = (0..b.visible.len())
        .filter_map(|i| b.entry(i))
        .map(|e| {
            if e.is_dir {
                ListItem::new(Line::styled(format!("▸ {}", e.name), Style::new().fg(Color::LightBlue)))
            } else {
                let ext = e.path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                let color = match ext.as_str() {
                    "sf2" => kind_color(Kind::Sf2),
                    "sfz" => kind_color(Kind::Sfz),
                    "wav" => kind_color(Kind::Wav),
                    _ => Color::LightYellow,
                };
                ListItem::new(Line::styled(format!("  {}", e.name), Style::new().fg(color)))
            }
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(Some(b.selected));
    f.render_stateful_widget(list, list_area, &mut state);
}

fn draw_ports(f: &mut Frame, app: &App, items: &[PortItem], sel: usize) {
    let area = popup_area(f.area(), 70, (items.len() as u16 + 6).max(10));
    f.render_widget(Clear, area);
    let out = app.midi.output_name();
    let thru = app.midi.thru.load(Ordering::Relaxed);
    let mut list_items = Vec::new();
    let mut last_kind = 255u8;
    let mut index_map = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let kind = match it {
            PortItem::Input(_) => 0,
            PortItem::Output(_) => 1,
            _ => 2,
        };
        if kind != last_kind {
            let header = ["MIDI inputs (Enter toggles)", "MIDI output (Enter selects)", "Options"][kind as usize];
            list_items.push(ListItem::new(Line::styled(header, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))));
            index_map.push(None);
            last_kind = kind;
        }
        let text = match it {
            PortItem::Input(n) => format!("  [{}] {n}", if app.midi.is_input_open(n) { "x" } else { " " }),
            PortItem::Output(None) => format!("  ({}) none", if out.is_none() { "•" } else { " " }),
            PortItem::Output(Some(n)) => format!("  ({}) {n}", if out.as_deref() == Some(n.as_str()) { "•" } else { " " }),
            PortItem::Thru => format!("  [{}] MIDI thru: inputs -> output", if thru { "x" } else { " " }),
            #[cfg(unix)]
            PortItem::Virtual => "  create ALSA virtual ports simpletui:in / simpletui:out".to_string(),
        };
        list_items.push(ListItem::new(text));
        index_map.push(Some(i));
    }
    let shown = index_map.iter().position(|m| *m == Some(sel));
    let list = List::new(list_items)
        .block(block("MIDI ports · r refresh · Esc close", true))
        .highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(shown);
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_devices(f: &mut Frame, app: &App, items: &[String], sel: usize) {
    let area = popup_area(f.area(), 70, (items.len() as u16 + 4).max(8));
    f.render_widget(Clear, area);
    let cur = app.audio.as_ref().map(|a| a.device.as_str());
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|d| ListItem::new(format!("({}) {d}", if Some(d.as_str()) == cur { "•" } else { " " })))
        .collect();
    let title = format!("Audio output · {} · Enter select", crate::audio::host_name());
    let list = List::new(list_items)
        .block(block(&title, true))
        .highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(Some(sel));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_presets(f: &mut Frame, app: &App, sel: usize, filter: &str, channel: Option<u8>) {
    let area = popup_area(f.area(), 60, 28);
    f.render_widget(Clear, area);
    let Some(s) = app.slots.get(app.sel) else { return };
    let list = app.filtered_presets(filter);
    let current = channel.map(|c| app.channel_info(app.sel, c as usize).preset).unwrap_or(s.preset);
    let target = match channel {
        Some(c) => format!("Preset for ch {}", c + 1),
        None => "Presets".to_string(),
    };
    let title = if filter.is_empty() { format!("{target} · type to filter") } else { format!("{target} · filter: {filter}") };
    let items: Vec<ListItem> = list
        .iter()
        .map(|&i| {
            let p = &s.inst.presets[i];
            let mark = if i == current { "•" } else { " " };
            ListItem::new(format!("{mark} {:03}:{:03}  {}  ({} zones)", p.bank, p.program, p.name, p.zones.len()))
        })
        .collect();
    let widget = List::new(items)
        .block(block(&title, true))
        .highlight_style(Style::new().bg(Color::Rgb(30, 50, 70)).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(list.iter().position(|&i| i == sel));
    f.render_stateful_widget(widget, area, &mut state);
}
