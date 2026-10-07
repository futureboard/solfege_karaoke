//! Audio output through cpal. cpal's default host is WASAPI on Windows and
//! ALSA on Linux, which are exactly the backends we target.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_channel::{Receiver, Sender};

use crate::engine::mixer::MAX_BUSES;
use crate::engine::{Command, Engine, Garbage, MAX_FRAMES, Shared};

pub struct AudioOut {
    _stream: Stream,
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    pub buffer: String,
}

pub fn host_name() -> String {
    cpal::default_host().id().name().to_string()
}

fn device_name(d: &Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| d.to_string())
}

pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|it| it.map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

pub fn default_device() -> Option<String> {
    cpal::default_host().default_output_device().map(|d| device_name(&d))
}

pub fn start(
    device: Option<&str>,
    buffer_frames: Option<u32>,
    rx: Receiver<Command>,
    garbage: Sender<Garbage>,
    shared: Arc<Shared>,
) -> Result<AudioOut> {
    let host = cpal::default_host();
    let dev = match device {
        Some(want) => host
            .output_devices()
            .context("enumerate output devices")?
            .find(|d| {
                let n = device_name(d);
                n == want || n.to_lowercase().contains(&want.to_lowercase())
            })
            .with_context(|| format!("no output device matching '{want}'"))?,
        None => host.default_output_device().context("no default output device")?,
    };
    let supported = dev.default_output_config().context("query default output config")?;
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.config();
    if let Some(n) = buffer_frames {
        config.buffer_size = BufferSize::Fixed(n);
    }

    let build = |config: StreamConfig| -> Result<Stream> {
        let engine = Engine::new(config.sample_rate as f32, rx.clone(), garbage.clone(), shared.clone());
        match format {
            SampleFormat::F32 => build_stream::<f32>(&dev, config, engine, shared.clone()),
            SampleFormat::I16 => build_stream::<i16>(&dev, config, engine, shared.clone()),
            SampleFormat::I32 => build_stream::<i32>(&dev, config, engine, shared.clone()),
            SampleFormat::U16 => build_stream::<u16>(&dev, config, engine, shared.clone()),
            SampleFormat::F64 => build_stream::<f64>(&dev, config, engine, shared.clone()),
            other => Err(anyhow!("unsupported sample format {other:?}")),
        }
    };
    // A fixed buffer size is a request; fall back to the device default.
    let stream = match build(config) {
        Ok(s) => s,
        Err(e) if buffer_frames.is_some() => {
            config.buffer_size = BufferSize::Default;
            build(config).map_err(|e2| anyhow!("{e:#}; retry with default buffer: {e2:#}"))?
        }
        Err(e) => return Err(e),
    };
    stream.play().context("start audio stream")?;
    let buffer = match stream.buffer_size() {
        Ok(n) => format!("{n} fr"),
        Err(_) => match config.buffer_size {
            BufferSize::Fixed(n) => format!("{n} fr"),
            BufferSize::Default => "default".into(),
        },
    };
    Ok(AudioOut {
        _stream: stream,
        host: host.id().name().to_string(),
        device: device_name(&dev),
        sample_rate: config.sample_rate,
        channels: config.channels,
        format: format!("{format:?}").to_lowercase(),
        buffer,
    })
}

fn build_stream<T>(dev: &Device, config: StreamConfig, mut engine: Engine, shared: Arc<Shared>) -> Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let sr = config.sample_rate as f64;
    // Each device channel pair is a mixer output bus (1/2 main, 3/4, ...).
    let pairs = (channels / 2).clamp(1, MAX_BUSES);
    engine.set_out_pairs(pairs);
    let err_shared = shared.clone();
    let stream = dev
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _| {
                let t0 = Instant::now();
                let frames = data.len() / channels.max(1);
                let mut done = 0;
                while done < frames {
                    let n = (frames - done).min(MAX_FRAMES);
                    engine.render(n);
                    let out = &mut data[done * channels..(done + n) * channels];
                    for (f, frame) in out.chunks_mut(channels).enumerate() {
                        if channels == 1 {
                            let (l, r) = engine.bus(0);
                            frame[0] = T::from_sample(0.5 * (l[f] + r[f]));
                            continue;
                        }
                        for (c, s) in frame.iter_mut().enumerate() {
                            let bus = c / 2;
                            *s = if bus < pairs {
                                let (l, r) = engine.bus(bus);
                                T::from_sample(if c % 2 == 0 { l[f] } else { r[f] })
                            } else {
                                T::from_sample(0.0)
                            };
                        }
                    }
                    done += n;
                }
                let budget = frames as f64 / sr;
                if budget > 0.0 {
                    let load = (t0.elapsed().as_secs_f64() / budget) as f32;
                    let prev = f32::from_bits(shared.cpu.load(Ordering::Relaxed));
                    shared.cpu.store((prev * 0.9 + load * 0.1).to_bits(), Ordering::Relaxed);
                }
            },
            move |_err| {
                err_shared.errors.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .context("build output stream")?;
    Ok(stream)
}
