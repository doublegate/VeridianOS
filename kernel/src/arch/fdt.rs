//! Minimal flattened device tree (FDT / DTB) reader.
//!
//! Enough to read a property by path -- e.g. `/cpus/timebase-frequency` and
//! the first CPU's ISA string on RISC-V -- without allocating. The blob comes
//! from firmware, so every offset and length in it is bounds-checked against
//! the blob; malformed input yields `None`, never a panic or an
//! out-of-bounds read.
//!
//! Format: devicetree specification v0.4, chapter 5 (big-endian header,
//! structure block of tokens, strings block).

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

/// A validated device tree blob.
pub struct Fdt<'a> {
    structs: &'a [u8],
    strings: &'a [u8],
}

fn be32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn align4(x: usize) -> Option<usize> {
    x.checked_add(3).map(|v| v & !3)
}

/// NUL-terminated string at `off` in `b`.
fn cstr(b: &[u8], off: usize) -> Option<&[u8]> {
    let rest = b.get(off..)?;
    let len = rest.iter().position(|&c| c == 0)?;
    Some(&rest[..len])
}

impl<'a> Fdt<'a> {
    /// Validate the header of `blob`.
    pub fn new(blob: &'a [u8]) -> Option<Self> {
        if be32(blob, 0)? != FDT_MAGIC {
            return None;
        }
        let total = be32(blob, 4)? as usize;
        let blob = blob.get(..total)?;
        let off_struct = be32(blob, 8)? as usize;
        let off_strings = be32(blob, 12)? as usize;
        let size_strings = be32(blob, 32)? as usize;
        let size_struct = be32(blob, 36)? as usize;
        Some(Self {
            structs: blob.get(off_struct..off_struct.checked_add(size_struct)?)?,
            strings: blob.get(off_strings..off_strings.checked_add(size_strings)?)?,
        })
    }

    /// The blob firmware left at physical address `pa` (0 = none).
    ///
    /// # Safety
    /// `pa` must be the address firmware passed for its device tree, mapped
    /// by the kernel's direct map, and the blob must stay in place.
    pub unsafe fn from_phys(pa: u64) -> Option<Fdt<'static>> {
        if pa == 0 {
            return None;
        }
        let base = crate::mm::phys_to_virt_addr(pa) as *const u8;
        // SAFETY: the caller guarantees a mapped blob; 40 bytes is the
        // fixed header, which is validated before anything else is read.
        let header = unsafe { core::slice::from_raw_parts(base, 40) };
        let size = Self::total_size(header)?.min(16 << 20);
        // SAFETY: totalsize from the validated header covers the blob.
        Fdt::new(unsafe { core::slice::from_raw_parts(base, size) })
    }

    /// Total size of a blob from its header, if it is one (to build the
    /// slice from a raw pointer).
    pub fn total_size(header: &[u8]) -> Option<usize> {
        (be32(header, 0)? == FDT_MAGIC).then(|| be32(header, 4).map(|v| v as usize))?
    }

    /// Value of property `prop` in the node at `path`. Each path component
    /// matches a node name exactly, or a node's base name (before `@`) when
    /// the component has no `@` -- so `["cpus", "cpu"]` finds the first
    /// `cpu@N`.
    pub fn property(&self, path: &[&str], prop: &str) -> Option<&'a [u8]> {
        let b = self.structs;
        let mut off = 0usize;
        // Depth of the node we are in, and how many leading path components
        // the current chain of open nodes matches.
        let mut depth = 0usize;
        let mut matched = 0usize;
        loop {
            let token = be32(b, off)?;
            off = off.checked_add(4)?;
            match token {
                FDT_BEGIN_NODE => {
                    let name = cstr(b, off)?;
                    off = align4(off.checked_add(name.len() + 1)?)?;
                    depth += 1;
                    // depth 1 is the root (empty name).
                    if depth >= 2 && matched == depth - 2 && depth - 2 < path.len() {
                        let want = path[depth - 2].as_bytes();
                        let base = name.split(|&c| c == b'@').next().unwrap_or(name);
                        let hit = name == want || (!want.contains(&b'@') && base == want);
                        if hit {
                            matched += 1;
                        }
                    }
                }
                FDT_END_NODE => {
                    if depth == 0 {
                        return None;
                    }
                    // The node closing here (index depth - 2 in the path) is
                    // on the matched chain iff matched >= depth - 1.
                    if depth >= 2 && matched >= depth - 1 {
                        if depth - 1 == path.len() {
                            return None; // target node had no such property
                        }
                        matched = depth - 2;
                    }
                    depth -= 1;
                }
                FDT_PROP => {
                    let len = be32(b, off)? as usize;
                    let nameoff = be32(b, off.checked_add(4)?)? as usize;
                    let val_off = off.checked_add(8)?;
                    let value = b.get(val_off..val_off.checked_add(len)?)?;
                    off = align4(val_off.checked_add(len)?)?;
                    if depth == path.len() + 1
                        && matched == path.len()
                        && cstr(self.strings, nameoff)? == prop.as_bytes()
                    {
                        return Some(value);
                    }
                }
                FDT_NOP => {}
                FDT_END => return None,
                _ => return None,
            }
        }
    }

    /// A property read as a big-endian u32 or u64 (cells of 1 or 2).
    pub fn property_u64(&self, path: &[&str], prop: &str) -> Option<u64> {
        let v = self.property(path, prop)?;
        match v.len() {
            4 => Some(u64::from(be32(v, 0)?)),
            8 => Some((u64::from(be32(v, 0)?) << 32) | u64::from(be32(v, 4)?)),
            _ => None,
        }
    }
}

/// Whether a RISC-V ISA description lists extension `ext` (ASCII,
/// case-insensitive): a `riscv,isa` string ("rv64imafdc_zicsr_sstc") or a
/// `riscv,isa-extensions` NUL-separated string list.
pub fn isa_has_extension(isa: &[u8], ext: &[u8]) -> bool {
    isa.split(|&c| c == b'_' || c == 0)
        .any(|e| e.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    /// Build a DTB: / { cpus { timebase-frequency = <10000000>;
    /// cpu@0 { riscv,isa = "rv64imafdc_sstc"; }; }; memory@80000000 { }; }
    fn sample() -> Vec<u8> {
        let mut strings = Vec::new();
        let tb = strings.len();
        strings.extend_from_slice(b"timebase-frequency\0");
        let isa = strings.len();
        strings.extend_from_slice(b"riscv,isa\0");

        let mut st = Vec::new();
        let tok = |st: &mut Vec<u8>, t: u32| st.extend_from_slice(&t.to_be_bytes());
        let name = |st: &mut Vec<u8>, n: &[u8]| {
            st.extend_from_slice(n);
            st.push(0);
            while st.len() % 4 != 0 {
                st.push(0);
            }
        };
        let prop = |st: &mut Vec<u8>, off: usize, v: &[u8]| {
            st.extend_from_slice(&FDT_PROP.to_be_bytes());
            st.extend_from_slice(&(v.len() as u32).to_be_bytes());
            st.extend_from_slice(&(off as u32).to_be_bytes());
            st.extend_from_slice(v);
            while st.len() % 4 != 0 {
                st.push(0);
            }
        };
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"");
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"memory@80000000");
        prop(&mut st, tb, &1u32.to_be_bytes()); // decoy: wrong node
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"cpus");
        prop(&mut st, tb, &10_000_000u32.to_be_bytes());
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"cpu@0");
        prop(&mut st, isa, b"rv64imafdc_sstc\0");
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END);

        let off_struct = 40usize;
        let off_strings = off_struct + st.len();
        let total = off_strings + strings.len();
        let mut b = Vec::new();
        for v in [
            FDT_MAGIC,
            total as u32,
            off_struct as u32,
            off_strings as u32,
            0,
            17,
            16,
            0,
            strings.len() as u32,
            st.len() as u32,
        ] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&st);
        b.extend_from_slice(&strings);
        b
    }

    #[test]
    fn finds_isa_extensions() {
        assert!(isa_has_extension(
            b"rv64imafdch_zicsr_zifencei_sstc_svadu\0",
            b"sstc"
        ));
        assert!(isa_has_extension(b"i\0m\0a\0sstc\0zicntr\0", b"sstc"));
        assert!(!isa_has_extension(b"rv64imafdc_zicsr\0", b"sstc"));
        assert!(!isa_has_extension(b"rv64imafdc_sstcx\0", b"sstc"));
    }

    #[test]
    fn reads_properties_by_path() {
        let blob = sample();
        let fdt = Fdt::new(&blob).unwrap();
        assert_eq!(
            fdt.property_u64(&["cpus"], "timebase-frequency"),
            Some(10_000_000)
        );
        assert_eq!(
            fdt.property(&["cpus", "cpu"], "riscv,isa"),
            Some(&b"rv64imafdc_sstc\0"[..])
        );
        assert_eq!(
            fdt.property(&["cpus", "cpu@0"], "riscv,isa").is_some(),
            true
        );
        assert_eq!(fdt.property(&["cpus"], "riscv,isa"), None);
        assert_eq!(fdt.property(&["nope"], "timebase-frequency"), None);
        assert_eq!(Fdt::total_size(&blob), Some(blob.len()));
    }

    #[test]
    fn rejects_malformed_blobs() {
        let blob = sample();
        // Truncated anywhere: never panics.
        for cut in 0..blob.len() {
            if let Some(fdt) = Fdt::new(&blob[..cut]) {
                let _ = fdt.property(&["cpus"], "timebase-frequency");
            }
        }
        // Bad magic.
        let mut bad = blob.clone();
        bad[0] = 0;
        assert!(Fdt::new(&bad).is_none());
        // Property length pointing past the structure block.
        let mut bad = blob.clone();
        let pos = bad
            .windows(4)
            .position(|w| w == FDT_PROP.to_be_bytes())
            .unwrap();
        bad[pos + 4..pos + 8].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
        let fdt = Fdt::new(&bad).unwrap();
        assert_eq!(fdt.property(&["cpus"], "timebase-frequency"), None);
    }
}
