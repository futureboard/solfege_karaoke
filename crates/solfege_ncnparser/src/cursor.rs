//! `.cur` files: an array of little-endian `u16` timestamps, one per lyric
//! byte (and per line break), in 1/24 of a quarter note.

use std::path::Path;

/// Cursor units per quarter note: `tick = value * ppq / CURSOR_RESOLUTION`.
pub const CURSOR_RESOLUTION: u32 = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub values: Vec<u16>,
    /// The file had an odd length; the last byte was dropped.
    pub truncated_byte: bool,
}

impl Cursor {
    pub fn parse(bytes: &[u8]) -> Self {
        let values = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Self { values, truncated_byte: bytes.len() % 2 == 1 }
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        Ok(Self::parse(&crate::error::read(path)?))
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Convert a cursor value to MIDI ticks.
    pub fn to_tick(value: u16, ppq: u16) -> u32 {
        (value as u64 * ppq as u64 / CURSOR_RESOLUTION as u64) as u32
    }

    /// How many entries step backwards in time (authoring jitter).
    pub fn backsteps(&self) -> usize {
        self.values.windows(2).filter(|w| w[1] < w[0]).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_convert() {
        let c = Cursor::parse(&[0x10, 0x00, 0x22, 0x01, 0x05, 0x00, 0xFF]);
        assert_eq!(c.values, [16, 290, 5]);
        assert!(c.truncated_byte);
        assert_eq!(c.backsteps(), 1);
        assert_eq!(Cursor::to_tick(24, 480), 480);
        assert_eq!(Cursor::to_tick(16, 480), 320);
    }
}
