use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    Io { path: PathBuf, source: std::io::Error },
    /// The MIDI file could not be read (bad header or chunk layout).
    InvalidMidi { path: Option<PathBuf>, reason: String },
    /// The library has no `Song`, `Lyrics` or `Cursor` folder.
    NotLibrary(PathBuf),
    NotFound(String),
    /// A song id exists but one of its three files is missing.
    MissingPart { id: String, part: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::InvalidMidi { path: Some(p), reason } => write!(f, "{}: invalid MIDI: {reason}", p.display()),
            Error::InvalidMidi { path: None, reason } => write!(f, "invalid MIDI: {reason}"),
            Error::NotLibrary(p) => write!(f, "{}: no Song/Lyrics/Cursor folders", p.display()),
            Error::NotFound(id) => write!(f, "song {id} not found"),
            Error::MissingPart { id, part } => write!(f, "song {id} has no {part} file"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn read(path: &std::path::Path) -> crate::Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })
}
