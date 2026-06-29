use crate::utilities::FileType::{Binary, Utf8Like, Utf16};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FileType {
    Utf8Like,
    Utf16 { little_endian: bool },
    Binary,
}

const TEST_WINDOW: usize = 8 * 1024;

/// Returns true if the data in question appears to be binary data (eg .png, .mp4, .mp3) or text data (.txt, .java etc)
/// This is not a bulletproof test, it can be wrong on very unusual binary files.
pub fn classify_file(bytes: &[u8]) -> FileType {
    if bytes.is_empty() {
        return Utf8Like;
    }

    // UTF-16 with a byte-order mark is text, even though it is full of NULs.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Utf16 {
            little_endian: true,
        };
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Utf16 {
            little_endian: false,
        };
    }

    // NULL byte
    let window = &bytes[..bytes.len().min(TEST_WINDOW)];
    if window.contains(&0) {
        return Binary;
    }

    // Last-ditch effort to find binary formats that don't have many NULL bytes by checking for control codes
    let control = window
        .iter()
        .filter(|&&b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t')
        .count();
    if control * 100 > window.len() {
        Binary
    } else {
        Utf8Like
    }
}

pub fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let rest = &bytes[2..]; // strip BOM
    let units: Vec<u16> = rest
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}
