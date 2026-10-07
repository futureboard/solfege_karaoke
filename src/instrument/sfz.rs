//! SFZ v1/v2 subset: <control> <global> <master> <group> <region>,
//! `#define`, `#include`, key/vel mapping, tuning, amp envelope, loops,
//! release triggers, choke groups, round robin (seq/rand), low-pass filter,
//! and the built-in `*sine` / `*saw` / `*square` / `*triangle` / `*noise` waves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use super::{
    Instrument, Kind, LoopMode, Preset, SampleData, Trigger, Zone, db_to_gain, parse_note, wav,
};

type Opcodes = HashMap<String, String>;
/// Decoded sample, its rate, `smpl` root key and loop points.
type Loaded = (Arc<SampleData>, f32, Option<u8>, Option<(usize, usize)>);

#[derive(Clone, Copy, PartialEq)]
enum Header {
    Control,
    Global,
    Master,
    Group,
    Region,
    Other,
}

struct Parser {
    root_dir: PathBuf,
    defines: HashMap<String, String>,
    default_path: String,
    note_offset: i32,
    octave_offset: i32,
    global: Opcodes,
    master: Opcodes,
    group: Opcodes,
    region: Option<Opcodes>,
    header: Header,
    regions: Vec<Opcodes>,
    warnings: Vec<String>,
    include_depth: usize,
}

pub fn load(path: &Path) -> Result<Instrument> {
    let text = read_text(path)?;
    let root_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut p = Parser {
        root_dir,
        defines: HashMap::new(),
        default_path: String::new(),
        note_offset: 0,
        octave_offset: 0,
        global: Opcodes::new(),
        master: Opcodes::new(),
        group: Opcodes::new(),
        region: None,
        header: Header::Other,
        regions: Vec::new(),
        warnings: Vec::new(),
        include_depth: 0,
    };
    p.parse_text(&text);
    p.close_region();

    let mut cache: HashMap<String, Option<Loaded>> = HashMap::new();
    let mut zones = Vec::new();
    let mut missing = 0usize;
    let mut sample_bytes = 0usize;
    for ops in &p.regions {
        let Some(sample_name) = ops.get("sample") else { continue };
        let entry = cache.entry(sample_name.clone()).or_insert_with(|| {
            let loaded = if let Some(builtin) = sample_name.strip_prefix('*') {
                builtin_wave(builtin).map(|s| {
                    let frames = s.frames;
                    (Arc::new(s), BUILTIN_RATE, None, Some((0, frames)))
                })
            } else {
                let file = resolve(&p.root_dir, &p.default_path, sample_name);
                match wav::read(&file) {
                    Ok(w) => Some((Arc::new(w.sample), w.sample_rate as f32, w.root, w.loop_points)),
                    Err(e) => {
                        p.warnings.push(format!("{sample_name}: {e:#}"));
                        None
                    }
                }
            };
            if let Some((s, ..)) = &loaded {
                sample_bytes += s.bytes();
            }
            loaded
        });
        let Some((sample, rate, wav_root, wav_loop)) = entry.clone() else {
            missing += 1;
            continue;
        };
        if let Some(z) = build_zone(ops, sample, rate, wav_root, wav_loop, p.note_offset + p.octave_offset * 12) {
            zones.push(z);
        }
    }
    if missing > 0 {
        p.warnings.insert(0, format!("{missing} region(s) skipped: sample not loaded"));
    }
    if zones.is_empty() {
        bail!(
            "no playable regions{}",
            p.warnings.first().map(|w| format!(" ({w})")).unwrap_or_default()
        );
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Instrument {
        name: name.clone(),
        kind: Kind::Sfz,
        path: path.to_path_buf(),
        presets: vec![Preset { name, bank: 0, program: 0, zones }],
        warnings: p.warnings,
        sample_bytes,
        wav: None,
    })
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn resolve(root: &Path, default_path: &str, name: &str) -> PathBuf {
    let rel = format!("{default_path}{name}").replace('\\', "/");
    let mut p = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map(|i| &r[i + 2..]).unwrap_or("");
            out.push(' ');
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map(|i| &r[i..]).unwrap_or("");
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

impl Parser {
    fn parse_text(&mut self, text: &str) {
        let text = strip_comments(text);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("#define") {
                let mut it = rest.split_whitespace();
                if let (Some(k), Some(v)) = (it.next(), it.next()) {
                    self.defines.insert(k.to_string(), v.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("#include") {
                let file = rest.trim().trim_matches('"');
                self.include(file);
                continue;
            }
            let line = self.expand(line);
            self.parse_line(&line);
        }
    }

    fn include(&mut self, file: &str) {
        if self.include_depth > 16 {
            self.warnings.push(format!("#include depth exceeded at {file}"));
            return;
        }
        let path = resolve(&self.root_dir, "", file);
        match read_text(&path) {
            Ok(text) => {
                self.include_depth += 1;
                self.parse_text(&text);
                self.include_depth -= 1;
            }
            Err(e) => self.warnings.push(format!("#include {file}: {e:#}")),
        }
    }

    fn expand(&self, line: &str) -> String {
        if !line.contains('$') || self.defines.is_empty() {
            return line.to_string();
        }
        // Longest names first so `$NOTE10` wins over `$NOTE1`.
        let mut defs: Vec<_> = self.defines.iter().collect();
        defs.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
        let mut s = line.to_string();
        for (k, v) in defs {
            s = s.replace(k.as_str(), v);
        }
        s
    }

    fn parse_line(&mut self, line: &str) {
        let mut rest = line;
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                return;
            }
            if let Some(r) = rest.strip_prefix('<') {
                let Some(end) = r.find('>') else { return };
                self.open_header(r[..end].trim());
                rest = &r[end + 1..];
                continue;
            }
            let Some(eq) = rest.find('=') else { return };
            let key = rest[..eq].trim().to_string();
            let after = &rest[eq + 1..];
            let (value, next) = split_value(after);
            self.set_opcode(key, value.trim().to_string());
            rest = next;
        }
    }

    fn open_header(&mut self, name: &str) {
        self.close_region();
        self.header = match name {
            "control" => Header::Control,
            "global" => {
                self.global.clear();
                self.master.clear();
                self.group.clear();
                Header::Global
            }
            "master" => {
                self.master.clear();
                self.group.clear();
                Header::Master
            }
            "group" => {
                self.group.clear();
                Header::Group
            }
            "region" => {
                self.region = Some(Opcodes::new());
                Header::Region
            }
            _ => Header::Other,
        };
    }

    fn close_region(&mut self) {
        if let Some(own) = self.region.take() {
            let mut ops = self.global.clone();
            ops.extend(self.master.iter().map(|(k, v)| (k.clone(), v.clone())));
            ops.extend(self.group.iter().map(|(k, v)| (k.clone(), v.clone())));
            ops.extend(own);
            self.regions.push(ops);
        }
    }

    fn set_opcode(&mut self, key: String, value: String) {
        match self.header {
            Header::Control => match key.as_str() {
                "default_path" => self.default_path = value.replace('\\', "/"),
                "note_offset" => self.note_offset = value.parse().unwrap_or(0),
                "octave_offset" => self.octave_offset = value.parse().unwrap_or(0),
                _ => {}
            },
            Header::Global => {
                self.global.insert(key, value);
            }
            Header::Master => {
                self.master.insert(key, value);
            }
            Header::Group => {
                self.group.insert(key, value);
            }
            Header::Region => {
                if let Some(r) = self.region.as_mut() {
                    r.insert(key, value);
                }
            }
            Header::Other => {}
        }
    }
}

/// Opcode values (notably sample paths) may contain spaces, so a value runs
/// until the next `name=` token or `<header>`.
fn split_value(s: &str) -> (&str, &str) {
    let header = s.find('<').unwrap_or(s.len());
    let mut search = 0;
    while let Some(off) = s[search..header].find('=') {
        let eq = search + off;
        let before = &s[..eq];
        let key_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        if key_start > 0 && key_start < eq {
            return (&s[..key_start], &s[key_start..]);
        }
        search = eq + 1;
    }
    (&s[..header], &s[header..])
}

fn num(ops: &Opcodes, key: &str) -> Option<f32> {
    ops.get(key).and_then(|v| v.trim().parse::<f32>().ok())
}

fn note(ops: &Opcodes, key: &str) -> Option<i32> {
    ops.get(key).and_then(|v| parse_note(v))
}

fn key_u8(v: i32, offset: i32) -> u8 {
    (v + offset).clamp(0, 127) as u8
}

fn build_zone(
    ops: &Opcodes,
    sample: Arc<SampleData>,
    rate: f32,
    wav_root: Option<u8>,
    wav_loop: Option<(usize, usize)>,
    key_offset: i32,
) -> Option<Zone> {
    let mut z = Zone::new(sample, rate);
    z.env.release = 0.001;

    if let Some(k) = note(ops, "key") {
        z.lokey = key_u8(k, key_offset);
        z.hikey = z.lokey;
        z.root = z.lokey as f32;
    }
    if let Some(k) = note(ops, "lokey") {
        z.lokey = key_u8(k, key_offset);
    }
    if let Some(k) = note(ops, "hikey") {
        if k < 0 {
            return None;
        }
        z.hikey = key_u8(k, key_offset);
    }
    match ops.get("pitch_keycenter").map(|s| s.trim()) {
        Some("sample") => z.root = wav_root.unwrap_or(60) as f32,
        Some(v) => {
            if let Some(k) = parse_note(v) {
                z.root = (k + key_offset) as f32;
            }
        }
        None => {}
    }
    if let Some(v) = num(ops, "lovel") {
        z.lovel = v.clamp(0.0, 127.0) as u8;
    }
    if let Some(v) = num(ops, "hivel") {
        z.hivel = v.clamp(0.0, 127.0) as u8;
    }
    z.lovel = z.lovel.max(1);

    let transpose = num(ops, "transpose").unwrap_or(0.0);
    z.tune = transpose * 100.0 + num(ops, "tune").or(num(ops, "pitch")).unwrap_or(0.0);
    if let Some(v) = num(ops, "pitch_keytrack") {
        z.keytrack = v;
    }

    let volume = num(ops, "volume").or(num(ops, "gain")).unwrap_or(0.0);
    let amplitude = num(ops, "amplitude").unwrap_or(100.0) / 100.0;
    z.gain = db_to_gain(volume) * amplitude;
    z.pan = (num(ops, "pan").unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0);
    z.veltrack = (num(ops, "amp_veltrack").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);

    z.env.delay = num(ops, "ampeg_delay").unwrap_or(0.0).max(0.0);
    z.env.attack = num(ops, "ampeg_attack").unwrap_or(0.0).max(0.0);
    z.env.hold = num(ops, "ampeg_hold").unwrap_or(0.0).max(0.0);
    z.env.decay = num(ops, "ampeg_decay").unwrap_or(0.0).max(0.0);
    z.env.sustain = (num(ops, "ampeg_sustain").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
    z.env.release = num(ops, "ampeg_release").unwrap_or(0.001).max(0.0);

    if let Some(v) = num(ops, "offset") {
        z.start = v.max(0.0) as usize;
    }
    if let Some(v) = num(ops, "end") {
        if v < 0.0 {
            return None;
        }
        z.end = v as usize + 1;
    }

    let (ls, le) = wav_loop.unwrap_or((0, z.end));
    z.loop_start = num(ops, "loop_start").or(num(ops, "loopstart")).map(|v| v as usize).unwrap_or(ls);
    z.loop_end = num(ops, "loop_end")
        .or(num(ops, "loopend"))
        .map(|v| v as usize + 1)
        .unwrap_or(le);
    z.loop_mode = match ops.get("loop_mode").or(ops.get("loopmode")).map(|s| s.trim()) {
        Some("no_loop") => LoopMode::NoLoop,
        Some("one_shot") => LoopMode::OneShot,
        Some("loop_continuous") => LoopMode::Continuous,
        Some("loop_sustain") => LoopMode::Sustain,
        _ if wav_loop.is_some() => LoopMode::Continuous,
        _ => LoopMode::NoLoop,
    };

    z.trigger = match ops.get("trigger").map(|s| s.trim()) {
        Some("release") | Some("release_key") => Trigger::Release,
        _ => Trigger::Attack,
    };
    if z.trigger == Trigger::Release && matches!(z.loop_mode, LoopMode::Continuous | LoopMode::Sustain) {
        z.loop_mode = LoopMode::NoLoop;
    }
    z.group = num(ops, "group").unwrap_or(0.0).max(0.0) as u32;
    z.off_by = num(ops, "off_by").unwrap_or(0.0).max(0.0) as u32;
    z.seq_length = num(ops, "seq_length").unwrap_or(1.0).max(1.0) as u32;
    z.seq_position = num(ops, "seq_position").unwrap_or(1.0).max(1.0) as u32;
    z.lorand = num(ops, "lorand").unwrap_or(0.0);
    z.hirand = num(ops, "hirand").unwrap_or(1.0);

    let fil_type = ops.get("fil_type").or(ops.get("filtype")).map(|s| s.trim().to_string());
    let is_lowpass = fil_type.as_deref().is_none_or(|t| t.starts_with("lpf"));
    if let Some(c) = num(ops, "cutoff")
        && is_lowpass
    {
        z.cutoff = Some(c.max(10.0));
        z.resonance_db = num(ops, "resonance").unwrap_or(0.0).clamp(0.0, 40.0);
    }

    z.sanitize();
    Some(z)
}

const BUILTIN_RATE: f32 = 2048.0 * 261.625_58;

/// One looped cycle of 2048 frames tuned so the root key (C4) plays at pitch.
fn builtin_wave(name: &str) -> Option<SampleData> {
    use std::f32::consts::TAU;
    const N: usize = 2048;
    let mut seed = 0x1234_5678u32;
    let data: Vec<f32> = (0..N)
        .map(|i| {
            let t = i as f32 / N as f32;
            match name {
                "sine" => (TAU * t).sin(),
                "saw" => 2.0 * t - 1.0,
                "square" => if t < 0.5 { 1.0 } else { -1.0 },
                "triangle" | "tri" => 1.0 - 4.0 * (t - 0.5).abs(),
                "noise" => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as f32 / u32::MAX as f32 * 2.0 - 1.0
                }
                "silence" => 0.0,
                _ => f32::NAN,
            }
        })
        .collect();
    if data[0].is_nan() {
        return None;
    }
    Some(SampleData::from_f32(1, data))
}
