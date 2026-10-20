// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Half-width katakana on the Linux console, for the Matrix rain.
//!
//! The console font holds at most 512 glyphs, and the stock ones have no
//! katakana. While Gantry runs it can swap in a patched copy of the current
//! font: the 56 glyphs U+FF66..=U+FF9D take over slots the login screen does
//! not need (Cyrillic, Greek, then accented Latin), and everything else stays
//! as it was. The original goes back on exit.
//!
//! Fonts move through kbd's `setfont` (`-O` to save, a path to load), so
//! Gantry needs no `ioctl` of its own. The parser and writer below handle
//! PSF1 and PSF2 with their Unicode tables.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// First half-width katakana code point.
pub const KANA_FIRST: u32 = 0xFF66;
/// Last half-width katakana code point.
pub const KANA_LAST: u32 = 0xFF9D;
/// Number of half-width katakana glyphs.
pub const KANA_COUNT: usize = 56;

/// GNU Unifont's 8x16 bitmaps for U+FF66..=U+FF9D.
const KANA_HEX: &str = include_str!("../assets/katakana-8x16.hex");

const PSF1_MAGIC: [u8; 2] = [0x36, 0x04];
const PSF2_MAGIC: [u8; 4] = [0x72, 0xb5, 0x4a, 0x86];
const PSF2_HEADER: usize = 32;

/// One Unicode table entry for a glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A code point drawn with this glyph.
    Char(char),
    /// A combining sequence drawn with this glyph.
    Seq(Vec<char>),
}

impl Entry {
    fn chars(&self) -> &[char] {
        match self {
            Self::Char(c) => std::slice::from_ref(c),
            Self::Seq(s) => s,
        }
    }
}

/// Why a font could not be read or patched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsfError {
    /// The data ended early.
    Truncated,
    /// Not a PSF1 or PSF2 font.
    BadMagic,
    /// A header value Gantry does not handle.
    Unsupported(&'static str),
    /// Too few glyphs the login screen can spare.
    NoRoom {
        /// Spare glyphs found.
        found: usize,
    },
}

impl fmt::Display for PsfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("console font is truncated"),
            Self::BadMagic => f.write_str("not a PSF console font"),
            Self::Unsupported(what) => write!(f, "unsupported console font: {what}"),
            Self::NoRoom { found } => write!(
                f,
                "console font has {found} spare glyphs; katakana need {KANA_COUNT}"
            ),
        }
    }
}

impl std::error::Error for PsfError {}

/// A console font with its Unicode table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    width: u32,
    height: u32,
    glyphs: Vec<Vec<u8>>,
    table: Vec<Vec<Entry>>,
}

fn take(data: &[u8], at: usize, len: usize) -> Result<&[u8], PsfError> {
    data.get(at..at.checked_add(len).ok_or(PsfError::Truncated)?)
        .ok_or(PsfError::Truncated)
}

fn le32(data: &[u8], at: usize) -> Result<u32, PsfError> {
    let b = take(data, at, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn to_usize(v: u32) -> Result<usize, PsfError> {
    usize::try_from(v).map_err(|_overflow| PsfError::Unsupported("size"))
}

/// Bytes per glyph row.
fn pitch(width: u32) -> usize {
    usize::try_from(width.div_ceil(8)).unwrap_or(usize::MAX)
}

impl Font {
    /// Reads a PSF1 or PSF2 font.
    ///
    /// # Errors
    /// [`PsfError`] when the data is not a font Gantry can handle.
    pub fn parse(data: &[u8]) -> Result<Self, PsfError> {
        if data.starts_with(&PSF2_MAGIC) {
            Self::parse_psf2(data)
        } else if data.starts_with(&PSF1_MAGIC) {
            Self::parse_psf1(data)
        } else {
            Err(PsfError::BadMagic)
        }
    }

    fn parse_psf1(data: &[u8]) -> Result<Self, PsfError> {
        let mode = *data.get(2).ok_or(PsfError::Truncated)?;
        let height = *data.get(3).ok_or(PsfError::Truncated)?;
        let count = if mode & 0x01 == 0 { 256 } else { 512 };
        let size = usize::from(height);
        if size == 0 {
            return Err(PsfError::Unsupported("zero height"));
        }
        let glyphs = take(data, 4, count * size)?
            .chunks(size)
            .map(<[u8]>::to_vec)
            .collect();
        let mut table = vec![Vec::new(); count];
        if mode & 0x06 != 0 {
            let mut at = 4 + count * size;
            for entries in &mut table {
                let mut seq: Option<Vec<char>> = None;
                loop {
                    let b = take(data, at, 2)?;
                    at += 2;
                    match u16::from_le_bytes([b[0], b[1]]) {
                        0xFFFF => break,
                        0xFFFE => {
                            if let Some(s) = seq.take().filter(|s| !s.is_empty()) {
                                entries.push(Entry::Seq(s));
                            }
                            seq = Some(Vec::new());
                        }
                        u => {
                            let c = char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}');
                            match &mut seq {
                                Some(s) => s.push(c),
                                None => entries.push(Entry::Char(c)),
                            }
                        }
                    }
                }
                if let Some(s) = seq.filter(|s| !s.is_empty()) {
                    entries.push(Entry::Seq(s));
                }
            }
        }
        Ok(Self {
            width: 8,
            height: u32::from(height),
            glyphs,
            table,
        })
    }

    fn parse_psf2(data: &[u8]) -> Result<Self, PsfError> {
        let header = to_usize(le32(data, 8)?)?;
        let flags = le32(data, 12)?;
        let count = to_usize(le32(data, 16)?)?;
        let size = to_usize(le32(data, 20)?)?;
        let height = le32(data, 24)?;
        let width = le32(data, 28)?;
        if !(1..=32).contains(&width) || !(1..=64).contains(&height) {
            return Err(PsfError::Unsupported("glyph size"));
        }
        if count == 0 || count > 512 {
            return Err(PsfError::Unsupported("glyph count"));
        }
        if size != pitch(width) * to_usize(height)? {
            return Err(PsfError::Unsupported("glyph stride"));
        }
        let glyphs = take(data, header, count * size)?
            .chunks(size)
            .map(<[u8]>::to_vec)
            .collect();
        let mut table = vec![Vec::new(); count];
        if flags & 0x01 != 0 {
            let mut rest = data
                .get(header + count * size..)
                .ok_or(PsfError::Truncated)?;
            for entries in &mut table {
                let end = rest
                    .iter()
                    .position(|&b| b == 0xFF)
                    .ok_or(PsfError::Truncated)?;
                let (record, tail) = rest.split_at(end);
                rest = tail.get(1..).unwrap_or_default();
                let mut parts = record.split(|&b| b == 0xFE);
                if let Some(singles) = parts.next() {
                    entries.extend(String::from_utf8_lossy(singles).chars().map(Entry::Char));
                }
                for part in parts.filter(|p| !p.is_empty()) {
                    entries.push(Entry::Seq(String::from_utf8_lossy(part).chars().collect()));
                }
            }
        }
        Ok(Self {
            width,
            height,
            glyphs,
            table,
        })
    }

    /// Writes the font as PSF2 with a Unicode table.
    #[must_use]
    pub fn to_psf2(&self) -> Vec<u8> {
        let size = self.glyphs.first().map_or(0, Vec::len);
        let mut out = Vec::with_capacity(PSF2_HEADER + self.glyphs.len() * (size + 4));
        out.extend_from_slice(&PSF2_MAGIC);
        for v in [
            0,
            32,
            1,
            u32::try_from(self.glyphs.len()).unwrap_or(0),
            u32::try_from(size).unwrap_or(0),
            self.height,
            self.width,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for g in &self.glyphs {
            out.extend_from_slice(g);
        }
        let mut utf8 = [0; 4];
        for entries in &self.table {
            for e in entries {
                if let Entry::Char(c) = e {
                    out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                }
            }
            for e in entries {
                if let Entry::Seq(s) = e {
                    out.push(0xFE);
                    for c in s {
                        out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                    }
                }
            }
            out.push(0xFF);
        }
        out
    }

    /// The glyph slot drawn for `c`, if the font maps it.
    #[must_use]
    pub fn slot_of(&self, c: char) -> Option<usize> {
        self.table
            .iter()
            .position(|entries| entries.contains(&Entry::Char(c)))
    }

    /// True when every half-width katakana has a glyph.
    #[must_use]
    pub fn has_kana(&self) -> bool {
        kana_chars().all(|c| self.slot_of(c).is_some())
    }

    /// Gives the 56 half-width katakana a glyph each, taking the slots the
    /// login screen can best spare. A font that already has them is left as is.
    ///
    /// # Errors
    /// [`PsfError::NoRoom`] when fewer than 56 slots map only to spare scripts.
    pub fn add_kana(&mut self) -> Result<(), PsfError> {
        if self.has_kana() {
            return Ok(());
        }
        let mut spare: Vec<(u8, usize)> = self
            .table
            .iter()
            .enumerate()
            .filter_map(|(slot, entries)| {
                let mut worst = None;
                for c in entries.iter().flat_map(Entry::chars) {
                    worst = worst.max(Some(spare_rank(*c)?));
                }
                worst.map(|rank| (rank, slot))
            })
            .collect();
        if spare.len() < KANA_COUNT {
            return Err(PsfError::NoRoom { found: spare.len() });
        }
        spare.sort_unstable();
        let bitmaps = kana_bitmaps().ok_or(PsfError::Unsupported("katakana asset"))?;
        for ((_, slot), (c, bits)) in spare.into_iter().zip(kana_chars().zip(bitmaps)) {
            self.glyphs[slot] = scale(&bits, self.width, self.height);
            self.table[slot] = vec![Entry::Char(c)];
        }
        Ok(())
    }
}

/// How readily a code point's glyph can go (lower first), or `None` when the
/// login screen may need it.
fn spare_rank(c: char) -> Option<u8> {
    match u32::from(c) {
        0x0400..=0x04FF => Some(0),                         // Cyrillic
        0x0370..=0x03FF => Some(1),                         // Greek
        0x0180..=0x024F => Some(2),                         // Latin Extended-B
        0x0100..=0x017F => Some(3),                         // Latin Extended-A
        0x00C0..=0x00FF if c != '×' && c != '÷' => Some(4), // Latin-1 letters
        _ => None,
    }
}

fn kana_chars() -> impl Iterator<Item = char> {
    (KANA_FIRST..=KANA_LAST).filter_map(char::from_u32)
}

/// The embedded 8x16 katakana, in code point order.
fn kana_bitmaps() -> Option<Vec<[u8; 16]>> {
    let mut out = Vec::with_capacity(KANA_COUNT);
    for line in KANA_HEX
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let (cp, hex) = line.split_once(':')?;
        let expected = KANA_FIRST + u32::try_from(out.len()).ok()?;
        if u32::from_str_radix(cp, 16).ok()? != expected || hex.len() != 32 {
            return None;
        }
        let mut rows = [0; 16];
        for (i, row) in rows.iter_mut().enumerate() {
            *row = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        out.push(rows);
    }
    (out.len() == KANA_COUNT).then_some(out)
}

/// Nearest-neighbour scale of an 8x16 bitmap to `width` x `height`.
fn scale(src: &[u8; 16], width: u32, height: u32) -> Vec<u8> {
    let p = pitch(width);
    let (w, h) = (
        usize::try_from(width).unwrap_or(8),
        usize::try_from(height).unwrap_or(16),
    );
    let mut out = vec![0; p * h];
    for y in 0..h {
        let row = src[y * 16 / h];
        for x in 0..w {
            if row & (0x80 >> (x * 8 / w)) != 0 {
                out[y * p + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

/// A patched font Gantry loaded, and how to put the original back.
#[derive(Debug, Clone)]
pub struct Loaded {
    setfont: PathBuf,
    original: PathBuf,
}

/// Runs `setfont` with `args`, folding failure into an error with its output.
fn setfont(program: &Path, args: &[&std::ffi::OsStr]) -> anyhow::Result<()> {
    let out = Command::new(program).args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

/// Saves the current console font in `dir`, then loads a copy with katakana.
///
/// The saved original stays in `dir` until [`Loaded::restore`] puts it back,
/// so a greeter killed mid-run still finds it on the next start.
///
/// # Errors
/// When `setfont` fails, `dir` is not writable, or the font has no room.
pub fn load(program: &Path, dir: &Path) -> anyhow::Result<Loaded> {
    let current = dir.join("console-font.current.psf");
    let original = dir.join("console-font.orig.psf");
    let patched = dir.join("console-font.gantry.psf");
    setfont(program, &["-O".as_ref(), current.as_os_str()])?;
    let mut font = Font::parse(&fs::read(&current)?)?;
    if font.has_kana() {
        // Already patched: an earlier run did not get to restore. The saved
        // original, when there is one, is still the font to go back to.
        let saved = fs::read(&original)
            .ok()
            .and_then(|d| Font::parse(&d).ok())
            .is_some_and(|f| !f.has_kana());
        let target = if saved { original } else { current };
        return Ok(Loaded {
            setfont: program.to_owned(),
            original: target,
        });
    }
    fs::rename(&current, &original)?;
    font.add_kana()?;
    fs::write(&patched, font.to_psf2())?;
    setfont(program, &[patched.as_os_str()])?;
    let _ = fs::remove_file(&patched);
    Ok(Loaded {
        setfont: program.to_owned(),
        original,
    })
}

impl Loaded {
    /// Loads the original font back and drops the saved copy.
    ///
    /// # Errors
    /// When `setfont` fails; the saved copy is then kept for the next run.
    pub fn restore(&self) -> anyhow::Result<()> {
        setfont(&self.setfont, &[self.original.as_os_str()])?;
        let _ = fs::remove_file(&self.original);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 256-glyph 8x16 font: slot i maps to `map(i)`.
    fn font(map: impl Fn(usize) -> Vec<Entry>) -> Font {
        Font {
            width: 8,
            height: 16,
            glyphs: (0..256)
                .map(|i| vec![u8::try_from(i).expect("byte"); 16])
                .collect(),
            table: (0..256).map(map).collect(),
        }
    }

    fn latin1(i: usize) -> Vec<Entry> {
        vec![Entry::Char(
            char::from_u32(u32::try_from(i).expect("u32")).expect("char"),
        )]
    }

    #[test]
    fn asset_has_all_56_glyphs() {
        let b = kana_bitmaps().expect("asset parses");
        assert_eq!(b.len(), KANA_COUNT);
        assert!(b.iter().all(|g| g.iter().any(|&r| r != 0)));
    }

    #[test]
    fn psf2_round_trips() {
        let mut f = font(latin1);
        f.table[65].push(Entry::Seq(vec!['A', '\u{0301}']));
        let back = Font::parse(&f.to_psf2()).expect("parses");
        assert_eq!(back, f);
    }

    #[test]
    fn psf1_with_table_parses() {
        let mut data = vec![0x36, 0x04, 0x02, 16];
        for i in 0..256u16 {
            data.extend(std::iter::repeat_n(u8::try_from(i).expect("byte"), 16));
        }
        for i in 0..256u16 {
            data.extend_from_slice(&i.to_le_bytes());
            if i == 65 {
                data.extend_from_slice(&0x0391u16.to_le_bytes()); // Greek Alpha
                data.extend_from_slice(&0xFFFEu16.to_le_bytes());
                data.extend_from_slice(&0x0041u16.to_le_bytes());
                data.extend_from_slice(&0x0301u16.to_le_bytes());
            }
            data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        }
        let f = Font::parse(&data).expect("parses");
        assert_eq!((f.width, f.height, f.glyphs.len()), (8, 16, 256));
        assert_eq!(f.slot_of('Α'), Some(65));
        assert_eq!(f.table[65].last(), Some(&Entry::Seq(vec!['A', '\u{0301}'])));
    }

    #[test]
    fn kana_take_only_spare_slots() {
        let mut f = font(latin1);
        f.add_kana().expect("room");
        assert!(f.has_kana());
        for c in (' '..='~').chain(['×', '÷', '£', '©', '·']) {
            assert!(f.slot_of(c).is_some(), "{c:?} kept");
        }
        // Latin-1 letters are the last resort, so some remain.
        assert!(f.slot_of('é').is_some() || f.slot_of('ÿ').is_some());
        assert_eq!(Font::parse(&f.to_psf2()).expect("parses"), f);
    }

    #[test]
    fn shared_slots_are_never_taken() {
        // Every slot also draws an ASCII letter: nothing is spare.
        let mut f = font(|i| {
            let mut e = latin1(i);
            e.push(Entry::Char('A'));
            e
        });
        assert_eq!(f.add_kana(), Err(PsfError::NoRoom { found: 0 }));
    }

    #[test]
    fn patched_font_is_left_alone() {
        let mut f = font(latin1);
        f.add_kana().expect("room");
        let once = f.clone();
        f.add_kana().expect("no-op");
        assert_eq!(f, once);
    }

    #[test]
    fn scaling_doubles_pixels() {
        let mut src = [0; 16];
        src[0] = 0b1000_0001;
        let out = scale(&src, 16, 32);
        assert_eq!(out.len(), 64);
        assert_eq!(
            &out[..4],
            &[0b1100_0000, 0b0000_0011, 0b1100_0000, 0b0000_0011]
        );
        assert!(out[4..].iter().all(|&b| b == 0));
        assert_eq!(scale(&src, 8, 16)[0], 0b1000_0001);
    }

    #[test]
    fn junk_is_rejected() {
        assert_eq!(Font::parse(b"nope"), Err(PsfError::BadMagic));
        assert_eq!(Font::parse(&PSF2_MAGIC), Err(PsfError::Truncated));
    }

    /// A fake `setfont` in a scratch dir: `-O FILE` writes the "console"
    /// font (the file `console.psf`) to FILE; `FILE` loads FILE as it.
    fn fake_setfont(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let script = dir.join("setfont");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -O ]; then cp {c} \"$2\"; else cp \"$1\" {c}; fi\n",
                c = dir.join("console.psf").display()
            ),
        )
        .expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("chmod");
        script
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gantry-vtfont-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn load_patches_then_restore_puts_the_original_back() {
        let dir = scratch("cycle");
        let setfont = fake_setfont(&dir);
        let original = font(latin1).to_psf2();
        fs::write(dir.join("console.psf"), &original).expect("seed");

        let loaded = load(&setfont, &dir).expect("loads");
        let live = Font::parse(&fs::read(dir.join("console.psf")).expect("read")).expect("parse");
        assert!(live.has_kana());
        assert!(
            dir.join("console-font.orig.psf").exists(),
            "kept for crash recovery"
        );

        loaded.restore().expect("restores");
        assert_eq!(fs::read(dir.join("console.psf")).expect("read"), original);
        assert!(!dir.join("console-font.orig.psf").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crashed_run_restores_the_saved_original() {
        let dir = scratch("crash");
        let setfont = fake_setfont(&dir);
        let original = font(latin1).to_psf2();
        fs::write(dir.join("console.psf"), &original).expect("seed");
        drop(load(&setfont, &dir).expect("loads")); // killed: no restore

        let again = load(&setfont, &dir).expect("loads again");
        again.restore().expect("restores");
        assert_eq!(fs::read(dir.join("console.psf")).expect("read"), original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn setfont_failure_is_an_error() {
        let dir = scratch("fail");
        load(Path::new("/nonexistent/setfont"), &dir).expect_err("no setfont");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Patches real kbd fonts when `GANTRY_PSF_DIR` points at uncompressed ones.
    #[test]
    fn real_fonts_have_room() {
        let Ok(dir) = std::env::var("GANTRY_PSF_DIR") else {
            return;
        };
        for entry in fs::read_dir(dir).expect("dir") {
            let path = entry.expect("entry").path();
            let mut f = Font::parse(&fs::read(&path).expect("read")).expect("parses");
            let result = f.add_kana();
            if result.is_ok() {
                fs::write(path.with_extension("gantry"), f.to_psf2()).expect("write");
            }
            eprintln!(
                "{}: {}x{} {} glyphs: {result:?}",
                path.display(),
                f.width,
                f.height,
                f.glyphs.len()
            );
        }
    }
}
