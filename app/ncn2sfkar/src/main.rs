//! ncn2sfkar: convert an NCN karaoke library (Song / Lyrics / Cursor
//! folders) into one `.sfkar` file per song.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use solfege_ncnparser::NcnLibrary;
use solfege_sfkar::{EXTENSION, KarSong, convert};

const USAGE: &str = "\
ncn2sfkar - convert an NCN karaoke library to .sfkar files

USAGE:
    ncn2sfkar <NCN_DIR> [OPTIONS] [SONG_ID...]

    NCN_DIR is the folder that holds Song, Lyrics and Cursor.
    With SONG_IDs only those songs are converted, otherwise all of them.

OPTIONS:
    -o, --out <DIR>   where to write the .sfkar files (default: <NCN_DIR>-sfkar)
    -f, --force       overwrite files that already exist
    -n, --dry-run     check every song converts, write nothing
    -q, --quiet       only print the summary and errors
    -h, --help        show this help
";

struct Args {
    root: PathBuf,
    out: Option<PathBuf>,
    ids: Vec<String>,
    force: bool,
    dry_run: bool,
    quiet: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut root = None;
    let mut a = Args { root: PathBuf::new(), out: None, ids: Vec::new(), force: false, dry_run: false, quiet: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-o" | "--out" => a.out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "-f" | "--force" => a.force = true,
            "-n" | "--dry-run" => a.dry_run = true,
            "-q" | "--quiet" => a.quiet = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n\n{USAGE}")),
            _ if root.is_none() => root = Some(PathBuf::from(arg)),
            _ => a.ids.push(arg),
        }
    }
    a.root = root.ok_or(format!("missing NCN_DIR\n\n{USAGE}"))?;
    Ok(Some(a))
}

/// `<NCN_DIR>-sfkar` next to the library.
fn default_out(root: &Path) -> PathBuf {
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "NCN".into());
    root.with_file_name(format!("{name}-sfkar"))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let lib = match NcnLibrary::open(&args.root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = args.out.clone().unwrap_or_else(|| default_out(&args.root));
    if !args.dry_run
        && let Err(e) = std::fs::create_dir_all(&out)
    {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::FAILURE;
    }

    let ids: Vec<String> = if args.ids.is_empty() {
        lib.entries().map(|e| e.id.clone()).collect()
    } else {
        args.ids.clone()
    };
    let started = Instant::now();
    let (mut written, mut skipped, mut failed, mut bytes) = (0usize, 0usize, 0usize, 0u64);
    for (n, id) in ids.iter().enumerate() {
        let entry = lib.get(id);
        let name = entry.map_or(id.as_str(), |e| e.id.as_str());
        let target = out.join(format!("{name}.{EXTENSION}"));
        if !args.force && !args.dry_run && target.exists() {
            skipped += 1;
            continue;
        }
        let result = convert(&lib, id).and_then(|kar| {
            // Read back what we would write: a file that does not decode is a bug.
            let data = kar.to_bytes();
            KarSong::parse(&data)?;
            if !args.dry_run {
                kar.save(&target)?;
            }
            Ok((kar, data.len()))
        });
        match result {
            Ok((kar, size)) => {
                written += 1;
                bytes += size as u64;
                if !args.quiet {
                    println!("[{}/{}] {name}  {} — {}", n + 1, ids.len(), kar.meta.title, kar.meta.artist);
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("[{}/{}] {name}: {e}", n + 1, ids.len());
            }
        }
    }
    let verb = if args.dry_run { "checked" } else { "converted" };
    println!(
        "{verb} {written} song(s), {:.1} MB, in {:.1}s{}{}{}",
        bytes as f64 / 1e6,
        started.elapsed().as_secs_f64(),
        if skipped > 0 { format!(", skipped {skipped} existing (use --force)") } else { String::new() },
        if failed > 0 { format!(", {failed} failed") } else { String::new() },
        if args.dry_run { String::new() } else { format!(" -> {}", out.display()) },
    );
    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
