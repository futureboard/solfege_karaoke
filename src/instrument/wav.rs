//! Minimal RIFF/WAVE reader: PCM 8/16/24/32-bit, IEEE float 32/64,
//! WAVE_FORMAT_EXTENSIBLE, plus the `smpl` chunk for root key and loops.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use super::{Envelope, Instrument, Kind, LoopMode, Preset, SampleData, WavParams, WavSource, Zone};

pub struct WavFile {
    pub sample: SampleData,
    pub sample_rate: u32,
    pub root: Option<u8>,
    pub fine_cents: f32,
    pub loop_points: Option<(usize, usize)>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn read(path: &Path) -> Result<WavFile> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    parse(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub fn parse(b: &[u8]) -> Result<WavFile> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    let mut fmt: Option<(u16, usize, u32, usize)> = None; // tag, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    let mut root = None;
    let mut fine_cents = 0.0;
    let mut loop_points = None;

    let mut pos = 12;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32_at(b, pos + 4) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(b.len());
        let body = &b[body_start..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                let mut tag = u16_at(body, 0);
                let channels = u16_at(body, 2) as usize;
                let rate = u32_at(body, 4);
                let bits = u16_at(body, 14) as usize;
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = u16_at(body, 24);
                }
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(body),
            b"smpl" if body.len() >= 36 => {
                let unity = u32_at(body, 12);
                if unity < 128 {
                    root = Some(unity as u8);
                }
                fine_cents = u32_at(body, 16) as f32 / 4_294_967_296.0 * 100.0;
                let loops = u32_at(body, 28) as usize;
                if loops > 0 && body.len() >= 36 + 24 {
                    let start = u32_at(body, 36 + 8) as usize;
                    let end = u32_at(body, 36 + 12) as usize;
                    if end > start {
                        // smpl loop end is inclusive.
                        loop_points = Some((start, end + 1));
                    }
                }
            }
            _ => {}
        }
        pos = body_start.saturating_add(size + (size & 1));
    }

    let (tag, channels, rate, bits) = fmt.context("missing fmt chunk")?;
    let data = data.context("missing data chunk")?;
    if channels == 0 {
        bail!("zero channels");
    }
    let bytes_per = bits.div_ceil(8);
    let frame_bytes = bytes_per * channels;
    let frames = data.len() / frame_bytes;
    let out_ch = channels.min(2);

    let sample = match (tag, bits) {
        (1, 16) => {
            let mut v = Vec::with_capacity(frames * out_ch);
            for f in 0..frames {
                for c in 0..out_ch {
                    let o = f * frame_bytes + c * 2;
                    v.push(i16::from_le_bytes([data[o], data[o + 1]]));
                }
            }
            SampleData::from_i16(out_ch, v)
        }
        (1, 8) | (1, 24) | (1, 32) | (3, 32) | (3, 64) => {
            let mut v = Vec::with_capacity(frames * out_ch);
            for f in 0..frames {
                for c in 0..out_ch {
                    let o = f * frame_bytes + c * bytes_per;
                    let s = match (tag, bits) {
                        (1, 8) => (data[o] as f32 - 128.0) / 128.0,
                        (1, 24) => {
                            let x = i32::from_le_bytes([0, data[o], data[o + 1], data[o + 2]]) >> 8;
                            x as f32 / 8_388_608.0
                        }
                        (1, 32) => {
                            let x = i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
                            x as f32 / 2_147_483_648.0
                        }
                        (3, 32) => f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
                        _ => {
                            let mut a = [0u8; 8];
                            a.copy_from_slice(&data[o..o + 8]);
                            f64::from_le_bytes(a) as f32
                        }
                    };
                    v.push(s);
                }
            }
            SampleData::from_f32(out_ch, v)
        }
        _ => bail!("unsupported WAV encoding (format tag {tag}, {bits} bits)"),
    };

    if sample.frames == 0 {
        bail!("no audio frames");
    }
    Ok(WavFile { sample, sample_rate: rate, root, fine_cents, loop_points })
}

pub fn load_instrument(path: &Path) -> Result<Instrument> {
    let wav = read(path)?;
    let looped = wav.loop_points.is_some();
    let params = WavParams {
        root: wav.root.unwrap_or(60),
        tune: wav.fine_cents,
        keytrack: true,
        loop_mode: if looped { LoopMode::Continuous } else { LoopMode::NoLoop },
        env: Envelope { release: 0.15, ..Envelope::default() },
    };
    let source = Arc::new(WavSource {
        sample: Arc::new(wav.sample),
        sample_rate: wav.sample_rate as f32,
        loop_points: wav.loop_points,
        params,
    });
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(build(name, path, source, params))
}

/// (Re)build a single-zone sampler instrument from a decoded WAV and parameters.
pub fn build(name: String, path: &Path, source: Arc<WavSource>, params: WavParams) -> Instrument {
    let mut zone = Zone::new(source.sample.clone(), source.sample_rate);
    zone.root = params.root as f32;
    zone.tune = params.tune;
    zone.keytrack = if params.keytrack { 100.0 } else { 0.0 };
    zone.loop_mode = params.loop_mode;
    if let Some((s, e)) = source.loop_points {
        zone.loop_start = s;
        zone.loop_end = e;
    }
    zone.env = params.env;
    zone.sanitize();
    let sample_bytes = source.sample.bytes();
    Instrument {
        name: name.clone(),
        kind: Kind::Wav,
        path: path.to_path_buf(),
        presets: vec![Preset { name, bank: 0, program: 0, zones: vec![zone] }],
        warnings: Vec::new(),
        sample_bytes,
        wav: Some(source),
    }
}
