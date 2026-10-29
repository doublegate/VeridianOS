//! veridian-elfctl: show, set or clear the VeridianOS feature-control note
//! of an ELF program, as FreeBSD's elfctl(1) does for its feature note.
//! The features:
//!
//! - `wxneeded` (ADR 0017): the program may map memory that is writable and
//!   executable at once;
//! - `linux-uname` (ADR 0021): uname reports the system name "Linux" to the
//!   program and what it runs.
//!
//! ```text
//! veridian-elfctl show FILE
//! veridian-elfctl set FEATURE FILE [-o OUT]
//! veridian-elfctl clear FEATURE FILE [-o OUT]
//! ```
//!
//! The kernel combines the feature words of every feature-control note, so
//! `show` reports them combined and `clear` clears the bit in each. A
//! program that already carries a note has its feature word changed in
//! place. A program without one -- any binary built for Linux -- gets
//! the note added without relinking, the way patchelf grows a program
//! header table:
//!
//! - a new program header table, the old entries plus the new ones, is written
//!   at the end of the file, followed by the note;
//! - a new PT_LOAD (read-only, above every other segment) maps the table and
//!   the note, so AT_PHDR still names a loaded table;
//! - a PT_NOTE names the note, and PT_PHDR (added first if the program had
//!   none, which musl's and glibc's loaders use to find their load base) is
//!   moved to the new table;
//! - `e_phoff` and `e_phnum` point at the new table. Everything else in the
//!   file is unchanged; the old table stays where it was, unused.
//!
//! Linux ignores the note, so the result still runs there. Run the tool
//! after `strip`, which rebuilds the file from its sections.

use std::{env, fs, path::Path, process::ExitCode};

const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const PT_PHDR: u32 = 6;
const PF_R: u32 = 4;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;
/// e_phnum's escape value (the real count would be in section 0).
const PN_XNUM: usize = 0xffff;

/// The owner and type of the VeridianOS feature-control note, and its
/// feature bits (`userland/libc/include/veridian/{wxneeded,linux-uname}.h`,
/// `kernel/src/elf/image.rs`).
const NOTE_NAME: &[u8] = b"VeridianOS\0";
const NT_VERIDIAN_FEATURE_CTL: u32 = 1;
const FCTL_WXNEEDED: u32 = 1;
const FCTL_LINUX_UNAME: u32 = 2;
const FEATURES: [(&str, u32); 2] = [
    ("wxneeded", FCTL_WXNEEDED),
    ("linux-uname", FCTL_LINUX_UNAME),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Phdr {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

impl Phdr {
    fn read(b: &[u8]) -> Phdr {
        Phdr {
            kind: u32_at(b, 0),
            flags: u32_at(b, 4),
            offset: u64_at(b, 8),
            vaddr: u64_at(b, 16),
            paddr: u64_at(b, 24),
            filesz: u64_at(b, 32),
            memsz: u64_at(b, 40),
            align: u64_at(b, 48),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.kind.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        for v in [
            self.offset,
            self.vaddr,
            self.paddr,
            self.filesz,
            self.memsz,
            self.align,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

// Callers check every offset against the data before reading it.
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(w)
}

/// The parts of an ELF file this tool reads, checked against its size.
struct Elf {
    phdrs: Vec<Phdr>,
}

fn parse(data: &[u8]) -> Result<Elf, String> {
    if data.len() < EHDR_SIZE || &data[..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if data[4] != 2 || data[5] != 1 {
        return Err("only 64-bit little-endian ELF files are supported".into());
    }
    let kind = u16_at(data, 16);
    if kind != ET_EXEC && kind != ET_DYN {
        return Err("not an executable (ET_EXEC or ET_DYN)".into());
    }
    let phoff = usize::try_from(u64_at(data, 32)).map_err(|_| "e_phoff out of range")?;
    let phentsize = usize::from(u16_at(data, 54));
    let phnum = usize::from(u16_at(data, 56));
    if phnum == PN_XNUM {
        return Err("more program headers than e_phnum can hold".into());
    }
    if phnum > 0 && phentsize != PHDR_SIZE {
        return Err(format!("e_phentsize is {phentsize}, not {PHDR_SIZE}"));
    }
    let end = phnum
        .checked_mul(PHDR_SIZE)
        .and_then(|n| n.checked_add(phoff))
        .filter(|&e| e <= data.len())
        .ok_or("program header table outside the file")?;
    let phdrs = data[phoff..end]
        .chunks_exact(PHDR_SIZE)
        .map(Phdr::read)
        .collect();
    Ok(Elf { phdrs })
}

/// The file offsets of the feature words of the VeridianOS feature-control
/// notes in every PT_NOTE segment, in file order. A malformed note segment
/// is an error, so the tool never rewrites what it cannot read.
fn find_feature_words(data: &[u8], elf: &Elf) -> Result<Vec<usize>, String> {
    let mut words = Vec::new();
    for p in elf.phdrs.iter().filter(|p| p.kind == PT_NOTE) {
        let bad = || format!("malformed PT_NOTE at offset {:#x}", p.offset);
        let start = usize::try_from(p.offset).map_err(|_| bad())?;
        let len = usize::try_from(p.filesz).map_err(|_| bad())?;
        let end = start
            .checked_add(len)
            .filter(|&e| e <= data.len())
            .ok_or_else(bad)?;
        let align = if p.align == 8 { 8 } else { 4 };
        let mut at = start;
        while at < end {
            if end - at < 12 {
                return Err(bad());
            }
            let namesz = u32_at(data, at) as usize;
            let descsz = u32_at(data, at + 4) as usize;
            let kind = u32_at(data, at + 8);
            let name_at = at + 12;
            let desc_at = name_at
                .checked_add(namesz)
                .and_then(|e| e.checked_next_multiple_of(align))
                .ok_or_else(bad)?;
            let next = desc_at
                .checked_add(descsz)
                .and_then(|e| e.checked_next_multiple_of(align))
                .ok_or_else(bad)?;
            if desc_at + descsz > end {
                return Err(bad());
            }
            if &data[name_at..name_at + namesz] == NOTE_NAME
                && kind == NT_VERIDIAN_FEATURE_CTL
                && descsz == 4
            {
                words.push(desc_at);
            }
            at = next;
        }
    }
    Ok(words)
}

/// The feature-control note with feature word `bits`, 4-byte aligned.
fn note_bytes(bits: u32) -> Vec<u8> {
    let mut n = Vec::with_capacity(28);
    n.extend_from_slice(&(NOTE_NAME.len() as u32).to_le_bytes());
    n.extend_from_slice(&4u32.to_le_bytes());
    n.extend_from_slice(&NT_VERIDIAN_FEATURE_CTL.to_le_bytes());
    n.extend_from_slice(NOTE_NAME);
    n.resize(n.len().next_multiple_of(4), 0);
    n.extend_from_slice(&bits.to_le_bytes());
    n
}

/// `data` with a feature-control note holding `bits` added, as the module
/// documentation describes.
fn add_note(data: &[u8], elf: &Elf, bits: u32) -> Result<Vec<u8>, String> {
    let loads: Vec<&Phdr> = elf.phdrs.iter().filter(|p| p.kind == PT_LOAD).collect();
    let last_load = elf
        .phdrs
        .iter()
        .rposition(|p| p.kind == PT_LOAD)
        .ok_or("no PT_LOAD segment")?;
    // The new segment's alignment: a page, or the largest a segment uses
    // (64 KiB on some arm64 binaries), so that offset and address agree.
    let mut page: u64 = 4096;
    for p in &loads {
        if p.align > 1 {
            if !p.align.is_power_of_two() || p.align > 1 << 30 {
                return Err(format!("PT_LOAD alignment {:#x} is not usable", p.align));
            }
            page = page.max(p.align);
        }
    }
    let top = loads
        .iter()
        .map(|p| p.vaddr.checked_add(p.memsz))
        .try_fold(0u64, |m, e| e.map(|e| m.max(e)))
        .ok_or("PT_LOAD end overflows")?;
    let has_phdr = elf.phdrs.iter().any(|p| p.kind == PT_PHDR);
    let count = elf.phdrs.len() + 2 + usize::from(!has_phdr);
    if count >= PN_XNUM {
        return Err("too many program headers".into());
    }

    let table_size = (count * PHDR_SIZE) as u64;
    let note = note_bytes(bits);
    let new_off = (data.len() as u64).next_multiple_of(page);
    let new_vaddr = top
        .checked_next_multiple_of(page)
        .ok_or("no room above the last segment")?;
    let seg_size = table_size + note.len() as u64;
    new_vaddr
        .checked_add(seg_size)
        .ok_or("no room above the last segment")?;
    let phdr = Phdr {
        kind: PT_PHDR,
        flags: PF_R,
        offset: new_off,
        vaddr: new_vaddr,
        paddr: new_vaddr,
        filesz: table_size,
        memsz: table_size,
        align: 8,
    };

    let mut table: Vec<Phdr> = Vec::with_capacity(count);
    if !has_phdr {
        table.push(phdr);
    }
    for (i, p) in elf.phdrs.iter().enumerate() {
        table.push(if p.kind == PT_PHDR { phdr } else { *p });
        if i == last_load {
            table.push(Phdr {
                kind: PT_LOAD,
                flags: PF_R,
                offset: new_off,
                vaddr: new_vaddr,
                paddr: new_vaddr,
                filesz: seg_size,
                memsz: seg_size,
                align: page,
            });
        }
    }
    table.push(Phdr {
        kind: PT_NOTE,
        flags: PF_R,
        offset: new_off + table_size,
        vaddr: new_vaddr + table_size,
        paddr: new_vaddr + table_size,
        filesz: note.len() as u64,
        memsz: note.len() as u64,
        align: 4,
    });
    debug_assert_eq!(table.len(), count);

    let mut out = data.to_vec();
    out[32..40].copy_from_slice(&new_off.to_le_bytes());
    out[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
    out[56..58].copy_from_slice(&(count as u16).to_le_bytes());
    out.resize(new_off as usize, 0);
    for p in &table {
        p.write(&mut out);
    }
    out.extend_from_slice(&note);
    Ok(out)
}

/// What a command does to the file for the feature `(name, bit)`: the new
/// contents, or `None` if it needs no change, with a line to report. `set`
/// sets the bit in the first note (adding one if there is none); `clear`
/// clears it in every note, as the kernel combines them.
fn apply(
    data: &[u8],
    set: bool,
    (name, bit): (&str, u32),
) -> Result<(Option<Vec<u8>>, String), String> {
    let elf = parse(data)?;
    let words = find_feature_words(data, &elf)?;
    if words.is_empty() {
        return Ok(if set {
            (
                Some(add_note(data, &elf, bit)?),
                "feature note added".into(),
            )
        } else {
            (None, format!("no feature note: {name} is not set"))
        });
    }
    let bits = combined(data, &words);
    if (bits & bit != 0) == set {
        return Ok((None, format!("{name} already as asked; unchanged")));
    }
    let mut out = data.to_vec();
    let targets = if set { &words[..1] } else { &words[..] };
    for &at in targets {
        let new = if set {
            u32_at(data, at) | bit
        } else {
            u32_at(data, at) & !bit
        };
        out[at..at + 4].copy_from_slice(&new.to_le_bytes());
    }
    Ok((Some(out), "feature note changed in place".into()))
}

/// The feature words at `words`, combined as the kernel combines them.
fn combined(data: &[u8], words: &[usize]) -> u32 {
    words.iter().fold(0, |b, &at| b | u32_at(data, at))
}

/// `show`: the features the program asks for, as the kernel reads them.
fn show(data: &[u8]) -> Result<String, String> {
    let elf = parse(data)?;
    let words = find_feature_words(data, &elf)?;
    let bits = combined(data, &words);
    let mut line = if words.is_empty() {
        "no feature note".to_string()
    } else {
        format!("feature note: {bits:#x}")
    };
    for (name, bit) in FEATURES {
        line += &format!("; {name}: {}", if bits & bit != 0 { "yes" } else { "no" });
    }
    Ok(line)
}

fn write_out(path: &Path, data: &[u8], mode_from: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let perms = fs::metadata(mode_from).map_err(err)?.permissions();
    // Write beside the target and rename, so a failure leaves it whole.
    let tmp = path.with_extension("elfctl-tmp");
    fs::write(&tmp, data).map_err(err)?;
    fs::set_permissions(&tmp, perms).map_err(err)?;
    fs::rename(&tmp, path).map_err(err)
}

const USAGE: &str = "usage: veridian-elfctl show FILE\n       veridian-elfctl set|clear \
                     wxneeded|linux-uname FILE [-o OUT]";

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [cmd, file] if cmd == "show" => show(&fs::read(file).map_err(|e| format!("{file}: {e}"))?),
        [cmd, feature, file, rest @ ..] if cmd == "set" || cmd == "clear" => {
            let Some(&known) = FEATURES.iter().find(|(name, _)| name == feature) else {
                return Err(format!(
                    "unknown feature {feature:?} (known: wxneeded, linux-uname)"
                ));
            };
            let out = match rest {
                [] => file.clone(),
                [flag, out] if flag == "-o" => out.clone(),
                _ => return Err(USAGE.into()),
            };
            let data = fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let (new, what) = apply(&data, cmd == "set", known)?;
            match new {
                Some(new) => write_out(Path::new(&out), &new, Path::new(file))?,
                None if out != *file => write_out(Path::new(&out), &data, Path::new(file))?,
                None => {}
            }
            Ok(format!("{out}: {what}"))
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("veridian-elfctl: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ET_DYN file: header, a table of `phdrs` at 64, then
    /// `body` bytes; the first PT_LOAD maps offset 0.
    fn elf(phdrs: &[Phdr], body: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; EHDR_SIZE];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 2;
        f[5] = 1;
        f[6] = 1;
        f[16..18].copy_from_slice(&ET_DYN.to_le_bytes());
        f[18..20].copy_from_slice(&62u16.to_le_bytes());
        f[32..40].copy_from_slice(&64u64.to_le_bytes());
        f[52..54].copy_from_slice(&64u16.to_le_bytes());
        f[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
        f[56..58].copy_from_slice(&(phdrs.len() as u16).to_le_bytes());
        for p in phdrs {
            p.write(&mut f);
        }
        f.extend_from_slice(body);
        f
    }

    fn load(offset: u64, vaddr: u64, size: u64, align: u64) -> Phdr {
        Phdr {
            kind: PT_LOAD,
            flags: PF_R | 1,
            offset,
            vaddr,
            paddr: vaddr,
            filesz: size,
            memsz: size + 0x100,
            align,
        }
    }

    const WX: (&str, u32) = FEATURES[0];
    const UNAME: (&str, u32) = FEATURES[1];

    /// The combined feature bits, or `None` without a note.
    fn feature_bits(data: &[u8]) -> Option<u32> {
        let elf = parse(data).unwrap();
        let words = find_feature_words(data, &elf).unwrap();
        (!words.is_empty()).then(|| combined(data, &words))
    }

    #[test]
    fn adds_a_note_with_a_new_mapped_table() {
        let phdr = Phdr {
            kind: PT_PHDR,
            flags: PF_R,
            offset: 64,
            vaddr: 64,
            paddr: 64,
            filesz: 3 * 56,
            memsz: 3 * 56,
            align: 8,
        };
        let old = vec![
            phdr,
            load(0, 0, 0x2000, 0x1000),
            load(0x2000, 0x3000, 0x500, 0x1000),
        ];
        let file = elf(&old, &[0xaa; 0x2600]);
        let (new, _) = apply(&file, true, WX).unwrap();
        let new = new.unwrap();
        assert_eq!(feature_bits(&new), Some(FCTL_WXNEEDED));
        // The old bytes are untouched past the header.
        assert_eq!(&new[EHDR_SIZE..file.len()], &file[EHDR_SIZE..]);
        let e = parse(&new).unwrap();
        assert_eq!(e.phdrs.len(), 5);
        let top = 0x3000 + 0x500 + 0x100;
        let at = (file.len() as u64).next_multiple_of(0x1000);
        let vaddr = (top as u64).next_multiple_of(0x1000);
        assert_eq!(u64_at(&new, 32), at);
        // PT_PHDR first and moved; the new PT_LOAD right after the last
        // one, mapping the table; the note last, inside it.
        assert_eq!(e.phdrs[0].kind, PT_PHDR);
        assert_eq!((e.phdrs[0].offset, e.phdrs[0].vaddr), (at, vaddr));
        assert_eq!(&e.phdrs[1..3], &old[1..3]);
        let l = e.phdrs[3];
        assert_eq!(
            (l.kind, l.offset, l.vaddr, l.align),
            (PT_LOAD, at, vaddr, 0x1000)
        );
        assert_eq!(l.filesz, 5 * 56 + 28);
        let n = e.phdrs[4];
        assert_eq!((n.kind, n.offset), (PT_NOTE, at + 5 * 56));
        assert_eq!(new.len() as u64, l.offset + l.filesz);
        // AT_PHDR, as the kernel derives it, is inside the new segment.
        assert!(e.phdrs[0].vaddr >= l.vaddr && e.phdrs[0].vaddr + 5 * 56 <= l.vaddr + l.memsz);
    }

    #[test]
    fn adds_pt_phdr_when_missing_and_honours_large_alignment() {
        let file = elf(&[load(0, 0x40_0000, 0x1000, 0x1_0000)], &[0; 0x1000]);
        let new = apply(&file, true, WX).unwrap().0.unwrap();
        let e = parse(&new).unwrap();
        assert_eq!(e.phdrs.len(), 4);
        assert_eq!(e.phdrs[0].kind, PT_PHDR);
        assert_eq!(e.phdrs[2].kind, PT_LOAD);
        assert_eq!(e.phdrs[2].align, 0x1_0000);
        assert_eq!(e.phdrs[2].offset % 0x1_0000, 0);
        assert_eq!(e.phdrs[2].vaddr % 0x1_0000, 0);
        assert_eq!(e.phdrs[0].vaddr, e.phdrs[2].vaddr);
    }

    #[test]
    fn flips_an_existing_note_in_place() {
        let note = note_bytes(0x10);
        let nphdr = Phdr {
            kind: PT_NOTE,
            flags: PF_R,
            offset: 64 + 2 * 56,
            vaddr: 64 + 2 * 56,
            paddr: 0,
            filesz: note.len() as u64,
            memsz: note.len() as u64,
            align: 4,
        };
        let file = elf(&[load(0, 0, 0x1000, 0x1000), nphdr], &note);
        let set = apply(&file, true, WX).unwrap().0.unwrap();
        assert_eq!(set.len(), file.len());
        assert_eq!(feature_bits(&set), Some(0x11));
        assert_eq!(apply(&set, true, WX).unwrap().0, None, "already set");
        let cleared = apply(&set, false, WX).unwrap().0.unwrap();
        assert_eq!(feature_bits(&cleared), Some(0x10), "other bits kept");
        assert!(show(&set).unwrap().contains("wxneeded: yes"));
        assert!(show(&cleared).unwrap().contains("wxneeded: no"));
    }

    #[test]
    fn clear_without_a_note_changes_nothing() {
        let file = elf(&[load(0, 0, 0x1000, 0x1000)], &[]);
        assert_eq!(apply(&file, false, WX).unwrap().0, None);
        assert_eq!(
            show(&file).unwrap(),
            "no feature note; wxneeded: no; linux-uname: no"
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        let good = elf(&[load(0, 0, 0x1000, 0x1000)], &[]);
        assert!(parse(&good[..40]).is_err(), "truncated header");
        let mut f = good.clone();
        f[4] = 1;
        assert!(parse(&f).is_err(), "32-bit");
        let mut f = good.clone();
        f[16] = 1;
        assert!(parse(&f).is_err(), "relocatable object");
        let mut f = good.clone();
        f[56..58].copy_from_slice(&50u16.to_le_bytes());
        assert!(parse(&f).is_err(), "table past the end");
        // A note segment that runs past the file.
        let bad_note = Phdr {
            kind: PT_NOTE,
            flags: PF_R,
            offset: 0x10_0000,
            vaddr: 0,
            paddr: 0,
            filesz: 28,
            memsz: 28,
            align: 4,
        };
        let f = elf(&[load(0, 0, 0x1000, 0x1000), bad_note], &[]);
        assert!(apply(&f, true, WX).is_err());
        // A note whose name size runs past its segment.
        let mut note = note_bytes(0);
        note[0..4].copy_from_slice(&0xffu32.to_le_bytes());
        let nphdr = Phdr {
            offset: 64 + 2 * 56,
            ..bad_note
        };
        let f = elf(&[load(0, 0, 0x1000, 0x1000), nphdr], &note);
        assert!(apply(&f, true, WX).is_err());
        // Nothing to map the new table with.
        let f = elf(&[nphdr], &note_bytes(0)[..0]);
        assert!(add_note(&f, &parse(&f).unwrap(), 1).is_err());
    }

    /// linux-uname shares the note: added on its own, set beside wxneeded
    /// in place, and cleared in every note the program carries.
    #[test]
    fn linux_uname_shares_the_feature_note() {
        let file = elf(&[load(0, 0, 0x2000, 0x1000)], &[0; 0x1000]);
        let added = apply(&file, true, UNAME).unwrap().0.unwrap();
        assert_eq!(feature_bits(&added), Some(FCTL_LINUX_UNAME));
        assert_eq!(
            show(&added).unwrap(),
            "feature note: 0x2; wxneeded: no; linux-uname: yes"
        );
        let both = apply(&added, true, WX).unwrap().0.unwrap();
        assert_eq!(both.len(), added.len(), "in place");
        assert_eq!(feature_bits(&both), Some(3));

        // Two notes, as a program with both headers carries: the bits are
        // combined, and clearing reaches the second.
        let mut notes = note_bytes(FCTL_WXNEEDED);
        notes.extend_from_slice(&note_bytes(FCTL_LINUX_UNAME));
        let nphdr = Phdr {
            kind: PT_NOTE,
            flags: PF_R,
            offset: 64 + 2 * 56,
            vaddr: 64 + 2 * 56,
            paddr: 0,
            filesz: notes.len() as u64,
            memsz: notes.len() as u64,
            align: 4,
        };
        let two = elf(&[load(0, 0, 0x1000, 0x1000), nphdr], &notes);
        assert_eq!(feature_bits(&two), Some(3));
        assert_eq!(apply(&two, true, UNAME).unwrap().0, None, "already set");
        let cleared = apply(&two, false, UNAME).unwrap().0.unwrap();
        assert_eq!(feature_bits(&cleared), Some(FCTL_WXNEEDED));
        assert!(show(&cleared).unwrap().ends_with("linux-uname: no"));
    }

    /// The note this tool writes is the one `<veridian/wxneeded.h>` emits.
    #[test]
    fn note_matches_the_header() {
        let mut want = Vec::new();
        for w in [11u32, 4, 1] {
            want.extend_from_slice(&w.to_le_bytes());
        }
        want.extend_from_slice(b"VeridianOS\0\0");
        want.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(note_bytes(1), want);
    }
}
