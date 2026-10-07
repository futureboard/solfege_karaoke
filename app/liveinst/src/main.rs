mod app;
mod browser;
mod midi;
mod web;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_player;
#[cfg(test)]
mod tests_channels;
#[cfg(test)]
mod tests_mixer;
mod ui;

// The sound engine lives in `solfege_synth`; importing its modules here
// keeps the `crate::engine::...` paths used across this app working.
use solfege_synth::{audio, engine, instrument, smf};

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ratatui::crossterm::event::{self, Event};

use app::{App, LoadTarget};

struct Args {
    files: Vec<PathBuf>,
    device: Option<String>,
    buffer: Option<u32>,
    midi_in: Vec<String>,
    midi_out: Option<String>,
    no_midi: bool,
    list: bool,
    play: bool,
    web: Option<std::net::SocketAddr>,
    web_dir: Option<PathBuf>,
    headless: bool,
}

const USAGE: &str = "\
simpletui - terminal instrument rack (WAV sampler, SFZ, SF2) with MIDI I/O

USAGE:
    simpletui [OPTIONS] [FILES...]

    FILES are instruments (.wav .sfz .sf2) and/or one MIDI file (.mid .kar .rmi).

OPTIONS:
    -d, --device <NAME>     audio output device (substring match)
    -b, --buffer <FRAMES>   request a fixed audio buffer size
    -i, --midi-in <NAME>    open this MIDI input (repeatable; default: all inputs)
    -o, --midi-out <NAME>   open this MIDI output
        --no-midi           do not auto-connect MIDI inputs
    -p, --play              start playing the MIDI file given in FILES
    -w, --web <ADDR>        web UI address (default 127.0.0.1:7878; port alone is fine)
        --web-dir <DIR>     web UI build directory (default: app/liveinst/webui/dist)
        --no-web            do not start the web UI
        --headless          no terminal UI: audio, MIDI and web UI only (Ctrl+C quits)
    -l, --list              list audio devices and MIDI ports, then exit
    -h, --help              show this help
";

fn parse_args() -> Result<Args> {
    let mut a = Args { files: Vec::new(), device: None, buffer: None, midi_in: Vec::new(), midi_out: None, no_midi: false,
        list: false,
        play: false,
        web: Some(std::net::SocketAddr::from(([127, 0, 0, 1], 7878))),
        web_dir: None,
        headless: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| anyhow::anyhow!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-d" | "--device" => a.device = Some(value("--device")?),
            "-b" | "--buffer" => a.buffer = Some(value("--buffer")?.parse()?),
            "-i" | "--midi-in" => a.midi_in.push(value("--midi-in")?),
            "-o" | "--midi-out" => a.midi_out = Some(value("--midi-out")?),
            "--no-midi" => a.no_midi = true,
            "-l" | "--list" => a.list = true,
            "-p" | "--play" => a.play = true,
            "-w" | "--web" => {
                let v = value("--web")?;
                let v = if v.contains(':') { v } else { format!("127.0.0.1:{v}") };
                a.web = Some(v.parse().map_err(|e| anyhow::anyhow!("--web {v}: {e}"))?);
            }
            "--web-dir" => a.web_dir = Some(PathBuf::from(value("--web-dir")?)),
            "--no-web" => a.web = None,
            "--headless" => a.headless = true,
            s if s.starts_with('-') => bail!("unknown option {s}\n\n{USAGE}"),
            _ => a.files.push(PathBuf::from(arg)),
        }
    }
    Ok(a)
}

fn list_devices() {
    println!("Audio host: {}", audio::host_name());
    let default = audio::default_device();
    for d in audio::output_devices() {
        let mark = if Some(&d) == default.as_ref() { "*" } else { " " };
        println!("  {mark} {d}");
    }
    println!("MIDI inputs:");
    for p in midi::input_ports() {
        println!("    {p}");
    }
    println!("MIDI outputs:");
    for p in midi::output_ports() {
        println!("    {p}");
    }
}

/// Key-up events: native on Windows consoles, via the kitty keyboard
/// protocol elsewhere. Without them the piano falls back to auto-release.
fn enable_key_release() -> bool {
    if cfg!(windows) {
        return true;
    }
    #[cfg(unix)]
    {
        use ratatui::crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        use ratatui::crossterm::{execute, terminal};
        if terminal::supports_keyboard_enhancement().unwrap_or(false) {
            return execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            )
            .is_ok();
        }
    }
    false
}

fn disable_key_release(enabled: bool) {
    #[cfg(unix)]
    if enabled {
        use ratatui::crossterm::event::PopKeyboardEnhancementFlags;
        let _ = ratatui::crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = enabled;
}

fn setup(args: &Args, release: bool) -> App {
    let mut app = App::new(release, args.buffer);

    app.start_audio(args.device.clone());
    if let Some(addr) = args.web {
        let dir = args.web_dir.clone().or_else(web::find_web_dir);
        app.start_web(addr, dir);
    }
    if args.midi_in.is_empty() {
        if !args.no_midi {
            app.connect_all_midi_inputs();
        }
    } else {
        for name in &args.midi_in {
            match app.midi.connect_input(name) {
                Ok(n) => app.info(format!("MIDI in: {n}")),
                Err(e) => app.error(format!("MIDI in: {e:#}")),
            }
        }
    }
    if let Some(name) = &args.midi_out {
        match app.midi.set_output(Some(name)) {
            Ok(Some(n)) => app.info(format!("MIDI out: {n}")),
            Ok(None) => {}
            Err(e) => app.error(format!("MIDI out: {e:#}")),
        }
    }
    let mut song = None;
    for f in &args.files {
        if smf::is_midi(f) {
            song = Some(f.clone());
        } else {
            app.load(f.clone(), LoadTarget::New);
        }
    }
    if let Some(f) = song
        && app.load_song(f)
        && args.play
    {
        // Starts once background instrument loads have finished.
        app.autoplay = true;
    }
    if !release && !args.headless {
        app.info("terminal has no key-up events: piano notes auto-release");
    }
    app
}


/// Audio + MIDI + web UI without a terminal; the log goes to stdout.
fn run_headless(mut app: App) -> Result<()> {
    let mut printed = 0usize;
    loop {
        app.tick();
        let total = app.log_total();
        if total > printed {
            for line in app.log.iter().skip(app.log.len().saturating_sub(total - printed)) {
                println!("{}{}", if line.error { "! " } else { "  " }, line.text);
            }
            printed = total;
        }
        if app.quit {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(33));
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.list {
        list_devices();
        return Ok(());
    }

    if args.headless {
        return run_headless(setup(&args, false));
    }
    let mut terminal = ratatui::init();
    let release = enable_key_release();
    let mut app = setup(&args, release);

    let frame = Duration::from_millis(33);
    let result = (|| -> Result<()> {
        let mut last = Instant::now();
        while !app.quit {
            terminal.draw(|f| ui::draw(f, &app))?;
            let timeout = frame.saturating_sub(last.elapsed());
            if event::poll(timeout)? {
                // Drain everything queued so key bursts stay responsive.
                loop {
                    if let Event::Key(k) = event::read()? {
                        app.on_key(k);
                    }
                    if app.quit || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
            if last.elapsed() >= frame {
                app.tick();
                last = Instant::now();
            }
        }
        Ok(())
    })();

    disable_key_release(release);
    ratatui::restore();
    result
}
