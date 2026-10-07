//! SoundFont 2 loader. Parses the RIFF `sfbk` structure (INFO, sdta, pdta),
//! resolves preset -> instrument -> sample zones with global zones and
//! additive preset generators, and flattens everything into `Zone`s.
//! Modulators are ignored except for the default velocity->amplitude curve,
//! which the engine models through `veltrack`.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use super::{Instrument, Kind, LoopMode, Preset, SampleData, Zone};

const START_ADDRS_OFFSET: usize = 0;
const END_ADDRS_OFFSET: usize = 1;
const STARTLOOP_ADDRS_OFFSET: usize = 2;
const ENDLOOP_ADDRS_OFFSET: usize = 3;
const START_ADDRS_COARSE: usize = 4;
const INITIAL_FILTER_FC: usize = 8;
const INITIAL_FILTER_Q: usize = 9;
const END_ADDRS_COARSE: usize = 12;
const PAN: usize = 17;
const DELAY_VOL_ENV: usize = 33;
const ATTACK_VOL_ENV: usize = 34;
const HOLD_VOL_ENV: usize = 35;
const DECAY_VOL_ENV: usize = 36;
const SUSTAIN_VOL_ENV: usize = 37;
const RELEASE_VOL_ENV: usize = 38;
const INSTRUMENT: usize = 41;
const KEY_RANGE: usize = 43;
const VEL_RANGE: usize = 44;
const STARTLOOP_ADDRS_COARSE: usize = 45;
const INITIAL_ATTENUATION: usize = 48;
const ENDLOOP_ADDRS_COARSE: usize = 50;
const COARSE_TUNE: usize = 51;
const FINE_TUNE: usize = 52;
const SAMPLE_ID: usize = 53;
const SAMPLE_MODES: usize = 54;
const SCALE_TUNING: usize = 56;
const EXCLUSIVE_CLASS: usize = 57;
const OVERRIDING_ROOT_KEY: usize = 58;
const GEN_COUNT: usize = 61;

/// Generators that are only legal at instrument level (spec 8.1.3).
const INSTRUMENT_ONLY: [usize; 13] = [
    START_ADDRS_OFFSET,
    END_ADDRS_OFFSET,
    STARTLOOP_ADDRS_OFFSET,
    ENDLOOP_ADDRS_OFFSET,
    START_ADDRS_COARSE,
    END_ADDRS_COARSE,
    STARTLOOP_ADDRS_COARSE,
    ENDLOOP_ADDRS_COARSE,
    46,
    47,
    SAMPLE_MODES,
    EXCLUSIVE_CLASS,
    OVERRIDING_ROOT_KEY,
];

#[derive(Clone, Copy)]
struct Gen {
    oper: u16,
    amount: [u8; 2],
}

impl Gen {
    fn i16(self) -> i16 {
        i16::from_le_bytes(self.amount)
    }
    fn range(self) -> (u8, u8) {
        (self.amount[0], self.amount[1])
    }
}

struct Bag {
    gen_ndx: usize,
}

struct PresetHeader {
    name: String,
    program: u16,
    bank: u16,
    bag_ndx: usize,
}

struct InstHeader {
    bag_ndx: usize,
}

struct SampleHeader {
    start: u32,
    end: u32,
    start_loop: u32,
    end_loop: u32,
    rate: u32,
    original_pitch: u8,
    pitch_correction: i8,
    sample_type: u16,
}

/// Generator set with explicit "is set" tracking so ranges can be merged.
#[derive(Clone)]
struct GenSet {
    vals: [i32; GEN_COUNT],
    set: [bool; GEN_COUNT],
}

impl GenSet {
    fn empty() -> Self {
        let mut g = Self { vals: [0; GEN_COUNT], set: [false; GEN_COUNT] };
        g.vals[KEY_RANGE] = 127 << 8;
        g.vals[VEL_RANGE] = 127 << 8;
        g
    }

    fn apply(&mut self, gens: &[Gen]) {
        for g in gens {
            let op = g.oper as usize;
            if op >= GEN_COUNT {
                continue;
            }
            self.vals[op] = match op {
                KEY_RANGE | VEL_RANGE => {
                    let (lo, hi) = g.range();
                    lo as i32 | ((hi as i32) << 8)
                }
                INSTRUMENT | SAMPLE_ID => u16::from_le_bytes(g.amount) as i32,
                _ => g.i16() as i32,
            };
            self.set[op] = true;
        }
    }

    fn range(&self, op: usize) -> (u8, u8) {
        let v = self.vals[op];
        ((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8)
    }
}

fn instrument_defaults() -> GenSet {
    let mut g = GenSet::empty();
    g.vals[INITIAL_FILTER_FC] = 13500;
    for op in [DELAY_VOL_ENV, ATTACK_VOL_ENV, HOLD_VOL_ENV, DECAY_VOL_ENV, RELEASE_VOL_ENV] {
        g.vals[op] = -12000;
    }
    g.vals[SCALE_TUNING] = 100;
    g.vals[OVERRIDING_ROOT_KEY] = -1;
    g.vals[46] = -1;
    g.vals[47] = -1;
    g
}

struct Reader<'a> {
    b: &'a [u8],
}

impl Reader<'_> {
    fn u16(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.b[o], self.b[o + 1]])
    }
    fn u32(&self, o: usize) -> u32 {
        u32::from_le_bytes([self.b[o], self.b[o + 1], self.b[o + 2], self.b[o + 3]])
    }
    fn name(&self, o: usize) -> String {
        let raw = &self.b[o..o + 20];
        let end = raw.iter().position(|&c| c == 0).unwrap_or(20);
        String::from_utf8_lossy(&raw[..end]).trim().to_string()
    }
}

/// Iterate (id, body) for sub-chunks inside a chunk body.
fn chunks(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 <= b.len() {
        let mut id = [0u8; 4];
        id.copy_from_slice(&b[pos..pos + 4]);
        let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let start = pos + 8;
        let end = start.saturating_add(size).min(b.len());
        out.push((id, &b[start..end]));
        pos = start.saturating_add(size + (size & 1));
    }
    out
}

fn records<T>(body: &[u8], size: usize, f: impl Fn(&Reader, usize) -> T) -> Vec<T> {
    let r = Reader { b: body };
    (0..body.len() / size).map(|i| f(&r, i * size)).collect()
}

pub fn load(path: &Path) -> Result<Instrument> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    parse(&bytes, path).with_context(|| format!("parse {}", path.display()))
}

fn parse(b: &[u8], path: &Path) -> Result<Instrument> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"sfbk" {
        bail!("not a SoundFont 2 file");
    }
    let mut bank_name = String::new();
    let mut smpl: &[u8] = &[];
    let mut sm24: &[u8] = &[];
    let mut pdta: Vec<([u8; 4], &[u8])> = Vec::new();

    for (id, body) in chunks(&b[12..]) {
        if &id != b"LIST" || body.len() < 4 {
            continue;
        }
        let kind = &body[0..4];
        let subs = chunks(&body[4..]);
        match kind {
            b"INFO" => {
                for (sid, sb) in &subs {
                    if sid == b"INAM" {
                        let end = sb.iter().position(|&c| c == 0).unwrap_or(sb.len());
                        bank_name = String::from_utf8_lossy(&sb[..end]).trim().to_string();
                    }
                }
            }
            b"sdta" => {
                for (sid, sb) in subs {
                    match &sid {
                        b"smpl" => smpl = sb,
                        b"sm24" => sm24 = sb,
                        _ => {}
                    }
                }
            }
            b"pdta" => pdta = subs,
            _ => {}
        }
    }
    let find = |name: &[u8; 4]| -> Result<&[u8]> {
        pdta.iter()
            .find(|(id, _)| id == name)
            .map(|(_, b)| *b)
            .with_context(|| format!("missing {} chunk", String::from_utf8_lossy(name)))
    };

    let phdr = records(find(b"phdr")?, 38, |r, o| PresetHeader {
        name: r.name(o),
        program: r.u16(o + 20),
        bank: r.u16(o + 22),
        bag_ndx: r.u16(o + 24) as usize,
    });
    let pbag = records(find(b"pbag")?, 4, |r, o| Bag { gen_ndx: r.u16(o) as usize });
    let pgen = records(find(b"pgen")?, 4, |r, o| Gen { oper: r.u16(o), amount: [r.b[o + 2], r.b[o + 3]] });
    let inst = records(find(b"inst")?, 22, |r, o| InstHeader { bag_ndx: r.u16(o + 20) as usize });
    let ibag = records(find(b"ibag")?, 4, |r, o| Bag { gen_ndx: r.u16(o) as usize });
    let igen = records(find(b"igen")?, 4, |r, o| Gen { oper: r.u16(o), amount: [r.b[o + 2], r.b[o + 3]] });
    let shdr = records(find(b"shdr")?, 46, |r, o| SampleHeader {
        start: r.u32(o + 20),
        end: r.u32(o + 24),
        start_loop: r.u32(o + 28),
        end_loop: r.u32(o + 32),
        rate: r.u32(o + 36),
        original_pitch: r.b[o + 40],
        pitch_correction: r.b[o + 41] as i8,
        sample_type: r.u16(o + 44),
    });
    if phdr.len() < 2 || inst.len() < 2 || shdr.is_empty() {
        bail!("SoundFont has no presets");
    }

    let frames = smpl.len() / 2;
    let sample = Arc::new(if sm24.len() >= frames {
        let data = (0..frames)
            .map(|i| {
                let hi = i16::from_le_bytes([smpl[2 * i], smpl[2 * i + 1]]) as i32;
                ((hi << 8) | sm24[i] as i32) as f32 / 8_388_608.0
            })
            .collect();
        SampleData::from_f32(1, data)
    } else {
        let data = (0..frames).map(|i| i16::from_le_bytes([smpl[2 * i], smpl[2 * i + 1]])).collect();
        SampleData::from_i16(1, data)
    });

    // Bag i covers generators [bag[i].gen_ndx, bag[i+1].gen_ndx).
    let bag_gens = |bags: &[Bag], gens: &'_ [Gen], i: usize| -> Vec<Gen> {
        let a = bags.get(i).map(|b| b.gen_ndx).unwrap_or(gens.len()).min(gens.len());
        let z = bags.get(i + 1).map(|b| b.gen_ndx).unwrap_or(gens.len()).min(gens.len());
        gens[a..z.max(a)].to_vec()
    };

    // Resolve each instrument into (generator set, sample index) zones.
    let mut inst_zones: Vec<Vec<(GenSet, usize)>> = Vec::with_capacity(inst.len() - 1);
    for i in 0..inst.len() - 1 {
        let (a, z) = (inst[i].bag_ndx, inst[i + 1].bag_ndx.min(ibag.len()));
        let mut global = instrument_defaults();
        let mut zones = Vec::new();
        for bi in a..z.max(a) {
            let gens = bag_gens(&ibag, &igen, bi);
            let has_sample = gens.last().is_some_and(|g| g.oper as usize == SAMPLE_ID);
            if !has_sample {
                if bi == a {
                    global.apply(&gens);
                }
                continue;
            }
            let mut set = global.clone();
            set.apply(&gens);
            zones.push((set.clone(), set.vals[SAMPLE_ID] as usize));
        }
        inst_zones.push(zones);
    }

    let mut warnings = Vec::new();
    let mut presets = Vec::new();
    for p in 0..phdr.len() - 1 {
        let h = &phdr[p];
        let (a, z) = (h.bag_ndx, phdr[p + 1].bag_ndx.min(pbag.len()));
        let mut global = GenSet::empty();
        let mut zones = Vec::new();
        for bi in a..z.max(a) {
            let gens = bag_gens(&pbag, &pgen, bi);
            let has_inst = gens.last().is_some_and(|g| g.oper as usize == INSTRUMENT);
            if !has_inst {
                if bi == a {
                    global.apply(&gens);
                }
                continue;
            }
            let mut pz = global.clone();
            pz.apply(&gens);
            let Some(izones) = inst_zones.get(pz.vals[INSTRUMENT] as usize) else { continue };
            for (iz, sample_id) in izones {
                let Some(sh) = shdr.get(*sample_id) else { continue };
                if sh.sample_type & 0x8000 != 0 {
                    continue; // ROM samples are not in the file
                }
                if let Some(zone) = make_zone(&pz, iz, sh, &sample) {
                    zones.push(zone);
                }
            }
        }
        if zones.is_empty() {
            continue;
        }
        presets.push(Preset {
            name: h.name.clone(),
            bank: h.bank,
            program: h.program.min(127) as u8,
            zones,
        });
    }
    if presets.is_empty() {
        bail!("no playable presets");
    }
    presets.sort_by_key(|p| (p.bank, p.program));
    if sm24.is_empty() && smpl.is_empty() {
        warnings.push("no sample data".to_string());
    }

    let name = if bank_name.is_empty() {
        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        bank_name
    };
    let sample_bytes = sample.bytes();
    Ok(Instrument {
        name,
        kind: Kind::Sf2,
        path: path.to_path_buf(),
        presets,
        warnings,
        sample_bytes,
        wav: None,
    })
}

fn timecents(tc: i32) -> f32 {
    if tc <= -12000 { 0.0 } else { 2f32.powf(tc.min(8000) as f32 / 1200.0) }
}

fn make_zone(pz: &GenSet, iz: &GenSet, sh: &SampleHeader, sample: &Arc<SampleData>) -> Option<Zone> {
    let (pkl, pkh) = pz.range(KEY_RANGE);
    let (ikl, ikh) = iz.range(KEY_RANGE);
    let (pvl, pvh) = pz.range(VEL_RANGE);
    let (ivl, ivh) = iz.range(VEL_RANGE);
    let (lokey, hikey) = (pkl.max(ikl), pkh.min(ikh));
    let (lovel, hivel) = (pvl.max(ivl), pvh.min(ivh));
    if lokey > hikey || lovel > hivel {
        return None;
    }

    // Instrument generators are absolute, preset generators add on top.
    let mut g = iz.vals;
    for (op, v) in g.iter_mut().enumerate() {
        if pz.set[op] && !INSTRUMENT_ONLY.contains(&op) && !matches!(op, KEY_RANGE | VEL_RANGE | INSTRUMENT) {
            *v += pz.vals[op];
        }
    }

    let off = |fine: usize, coarse: usize| g[fine] as i64 + g[coarse] as i64 * 32768;
    let clampi = |v: i64| v.clamp(0, sample.frames as i64) as usize;
    let mut z = Zone::new(sample.clone(), sh.rate.max(1) as f32);
    z.start = clampi(sh.start as i64 + off(START_ADDRS_OFFSET, START_ADDRS_COARSE));
    z.end = clampi(sh.end as i64 + off(END_ADDRS_OFFSET, END_ADDRS_COARSE));
    z.loop_start = clampi(sh.start_loop as i64 + off(STARTLOOP_ADDRS_OFFSET, STARTLOOP_ADDRS_COARSE));
    z.loop_end = clampi(sh.end_loop as i64 + off(ENDLOOP_ADDRS_OFFSET, ENDLOOP_ADDRS_COARSE));
    if z.end <= z.start {
        return None;
    }
    z.lokey = lokey;
    z.hikey = hikey.min(127);
    z.lovel = lovel.max(1);
    z.hivel = hivel.min(127);

    let root = if g[OVERRIDING_ROOT_KEY] >= 0 {
        g[OVERRIDING_ROOT_KEY]
    } else if sh.original_pitch <= 127 {
        sh.original_pitch as i32
    } else {
        60
    };
    z.root = root as f32;
    z.tune = g[COARSE_TUNE] as f32 * 100.0 + g[FINE_TUNE] as f32 + sh.pitch_correction as f32;
    z.keytrack = g[SCALE_TUNING] as f32;

    // EMU-style 0.4 attenuation scaling, as used by FluidSynth.
    let atten_cb = (g[INITIAL_ATTENUATION].clamp(0, 1440) as f32) * 0.4;
    z.gain = 10f32.powf(-atten_cb / 200.0);
    z.pan = (g[PAN].clamp(-500, 500) as f32) / 500.0;
    z.veltrack = 1.0;

    z.env.delay = timecents(g[DELAY_VOL_ENV]);
    z.env.attack = timecents(g[ATTACK_VOL_ENV]);
    z.env.hold = timecents(g[HOLD_VOL_ENV]);
    z.env.decay = timecents(g[DECAY_VOL_ENV]);
    z.env.sustain = 10f32.powf(-(g[SUSTAIN_VOL_ENV].clamp(0, 1440) as f32) / 200.0);
    z.env.release = timecents(g[RELEASE_VOL_ENV]).max(0.005);

    z.loop_mode = match g[SAMPLE_MODES] & 3 {
        1 => LoopMode::Continuous,
        3 => LoopMode::Sustain,
        _ => LoopMode::NoLoop,
    };
    let class = g[EXCLUSIVE_CLASS].max(0) as u32;
    z.group = class;
    z.off_by = class;

    let fc = g[INITIAL_FILTER_FC].clamp(1500, 13500);
    if fc < 13500 {
        z.cutoff = Some(8.176 * 2f32.powf(fc as f32 / 1200.0));
        z.resonance_db = g[INITIAL_FILTER_Q].clamp(0, 960) as f32 / 10.0;
    }

    z.sanitize();
    Some(z)
}
