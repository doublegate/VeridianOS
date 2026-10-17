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
    /// The memory reservation block (pairs of big-endian u64 address and
    /// size, ended by a pair of zeros), to the end of the blob.
    rsvmap: &'a [u8],
    /// The blob's total size (it must not be reused while it is read).
    total: usize,
}

/// One step of [`Fdt::walk`]. Depth 1 is the root node.
enum Event<'a> {
    Begin {
        depth: usize,
        name: &'a [u8],
    },
    Prop {
        depth: usize,
        name: &'a [u8],
        value: &'a [u8],
    },
    End {
        depth: usize,
    },
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
        let off_rsvmap = be32(blob, 16)? as usize;
        Some(Self {
            structs: blob.get(off_struct..off_struct.checked_add(size_struct)?)?,
            strings: blob.get(off_strings..off_strings.checked_add(size_strings)?)?,
            rsvmap: blob.get(off_rsvmap..)?,
            total,
        })
    }

    /// The blob's size in bytes (its header's totalsize).
    pub fn size(&self) -> usize {
        self.total
    }

    /// Call `f` for each token of the structure block, in order. Stops
    /// quietly at malformed data.
    fn walk(&self, mut f: impl FnMut(Event<'a>)) {
        let b = self.structs;
        let mut off = 0usize;
        let mut depth = 0usize;
        let mut step = || -> Option<bool> {
            let token = be32(b, off)?;
            off = off.checked_add(4)?;
            match token {
                FDT_BEGIN_NODE => {
                    let name = cstr(b, off)?;
                    off = align4(off.checked_add(name.len() + 1)?)?;
                    depth += 1;
                    f(Event::Begin { depth, name });
                }
                FDT_END_NODE => {
                    if depth == 0 {
                        return None;
                    }
                    f(Event::End { depth });
                    depth -= 1;
                }
                FDT_PROP => {
                    let len = be32(b, off)? as usize;
                    let nameoff = be32(b, off.checked_add(4)?)? as usize;
                    let val_off = off.checked_add(8)?;
                    let value = b.get(val_off..val_off.checked_add(len)?)?;
                    off = align4(val_off.checked_add(len)?)?;
                    let name = cstr(self.strings, nameoff)?;
                    f(Event::Prop { depth, name, value });
                }
                FDT_NOP => {}
                FDT_END => return Some(false),
                _ => return None,
            }
            Some(true)
        };
        while let Some(true) = step() {}
    }

    /// The root's `#address-cells` and `#size-cells` (2 and 1 when absent,
    /// as the specification says), each 1 or 2.
    fn root_cells(&self) -> Option<(usize, usize)> {
        let get = |p: &str, default: u64| match self.root_property(p) {
            Some(v) => cells_u64(v),
            None => Some(default),
        };
        let ac = get("#address-cells", 2)? as usize;
        let sc = get("#size-cells", 1)? as usize;
        ((1..=2).contains(&ac) && (1..=2).contains(&sc)).then_some((ac, sc))
    }

    /// A property of the root node itself.
    fn root_property(&self, prop: &str) -> Option<&'a [u8]> {
        let mut found = None;
        self.walk(|e| {
            if let Event::Prop {
                depth: 1,
                name,
                value,
            } = e
            {
                if name == prop.as_bytes() && found.is_none() {
                    found = Some(value);
                }
            }
        });
        found
    }

    /// Call `f(base, size)` for each range of every `reg` entry of the
    /// top-level memory nodes (named `memory` or `memory@...`, or with
    /// `device_type = "memory"`), in the root's address and size cells.
    /// Nothing is reported for cell sizes other than 1 or 2.
    pub fn memory_ranges(&self, mut f: impl FnMut(u64, u64)) {
        let Some(cells) = self.root_cells() else {
            return;
        };
        let mut memory = false;
        let mut reg: Option<&'a [u8]> = None;
        self.walk(|e| match e {
            Event::Begin { depth: 2, name } => {
                let base = name.split(|&c| c == b'@').next().unwrap_or(name);
                memory = base == b"memory";
                reg = None;
            }
            Event::Prop {
                depth: 2,
                name,
                value,
            } => match name {
                b"reg" => reg = Some(value),
                b"device_type" => memory |= trim_nul(value) == b"memory",
                _ => {}
            },
            Event::End { depth: 2 } => {
                if memory {
                    if let Some(r) = reg {
                        reg_ranges(r, cells, &mut f);
                    }
                }
            }
            _ => {}
        });
    }

    /// The first `reg` range (base, size) of the first enabled node whose
    /// `compatible` list names `compat`, read in its parent's address and
    /// size cells. Only identity bus mappings are followed: a node under
    /// a bus with a non-empty `ranges` (an address translation, which this
    /// reader does not apply) is not reported.
    pub fn compatible_reg(&self, compat: &[u8]) -> Option<(u64, u64)> {
        /// Deeper nodes are not considered (the specification sets no
        /// limit; real trees nest a handful of levels).
        const MAX_DEPTH: usize = 16;
        #[derive(Clone, Copy)]
        struct Node<'a> {
            /// `#address-cells`, `#size-cells` for this node's children.
            cells: (usize, usize),
            /// Whether its children's addresses are this bus's own (no
            /// `ranges` translation between them and the root).
            identity: bool,
            hit: bool,
            enabled: bool,
            reg: Option<&'a [u8]>,
        }
        let blank = Node {
            cells: (2, 1),
            identity: true,
            hit: false,
            enabled: true,
            reg: None,
        };
        let mut nodes = [blank; MAX_DEPTH + 1];
        let mut found = None;
        self.walk(|e| match e {
            Event::Begin { depth, .. } if depth <= MAX_DEPTH => {
                // Properties precede subnodes, so the parent's are known.
                let parent_identity = depth < 2 || nodes[depth - 1].identity;
                nodes[depth] = Node {
                    identity: parent_identity,
                    ..blank
                };
            }
            Event::Prop { depth, name, value } if depth <= MAX_DEPTH => {
                let n = &mut nodes[depth];
                match name {
                    b"#address-cells" => {
                        if let Some(c) = cells_u64(value) {
                            n.cells.0 = c as usize;
                        }
                    }
                    b"#size-cells" => {
                        if let Some(c) = cells_u64(value) {
                            n.cells.1 = c as usize;
                        }
                    }
                    b"ranges" => n.identity &= value.is_empty(),
                    b"compatible" => n.hit = value.split(|&c| c == 0).any(|s| s == compat),
                    b"status" => {
                        let s = trim_nul(value);
                        n.enabled = s == b"okay" || s == b"ok";
                    }
                    b"reg" => n.reg = Some(value),
                    _ => {}
                }
            }
            Event::End { depth } if (2..=MAX_DEPTH).contains(&depth) && found.is_none() => {
                let n = nodes[depth];
                let parent = nodes[depth - 1];
                let (ac, sc) = parent.cells;
                if n.hit
                    && n.enabled
                    && parent.identity
                    && (1..=2).contains(&ac)
                    && (1..=2).contains(&sc)
                {
                    if let Some(r) = n.reg {
                        reg_ranges(r, (ac, sc), &mut |b, s| {
                            found.get_or_insert((b, s));
                        });
                    }
                }
            }
            _ => {}
        });
        found
    }

    /// Call `f(base, size)` for every range firmware reserved: the memory
    /// reservation block, then each child of `/reserved-memory` with a
    /// `reg` (in that node's cells, the root's when it has none). Children
    /// with only a `size` (to be allocated by the OS) are not reported.
    pub fn reserved_ranges(&self, mut f: impl FnMut(u64, u64)) {
        let mut off = 0usize;
        while let (Some(addr), Some(size)) = (be64(self.rsvmap, off), be64(self.rsvmap, off + 8)) {
            if addr == 0 && size == 0 {
                break;
            }
            f(addr, size);
            off += 16;
        }
        let Some(root) = self.root_cells() else {
            return;
        };
        let mut in_reserved = false;
        let mut cells = root;
        let mut reg: Option<&'a [u8]> = None;
        self.walk(|e| match e {
            Event::Begin { depth: 2, name } => {
                in_reserved = name == b"reserved-memory";
                cells = root;
            }
            Event::Prop {
                depth: 2,
                name,
                value,
            } if in_reserved => {
                let n = cells_u64(value).map(|v| v as usize);
                match (name, n) {
                    (b"#address-cells", Some(n @ 1..=2)) => cells.0 = n,
                    (b"#size-cells", Some(n @ 1..=2)) => cells.1 = n,
                    _ => {}
                }
            }
            Event::Begin { depth: 3, .. } => reg = None,
            Event::Prop {
                depth: 3,
                name: b"reg",
                value,
            } if in_reserved => reg = Some(value),
            Event::End { depth: 3 } => {
                if let Some(r) = reg.take().filter(|_| in_reserved) {
                    reg_ranges(r, cells, &mut f);
                }
            }
            Event::End { depth: 2 } => in_reserved = false,
            _ => {}
        });
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

/// One `/cpus/cpu@N` node, for CPU enumeration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuNode<'a> {
    /// `reg`: hart id (RISC-V) or MPIDR affinity (AArch64).
    pub reg: Option<u64>,
    /// `status`, when present ("okay", "disabled", ...), without the NUL.
    pub status: Option<&'a [u8]>,
    /// `enable-method` ("psci", "spin-table"), without the NUL.
    pub enable_method: Option<&'a [u8]>,
    /// `mmu-type` (RISC-V), without the NUL.
    pub mmu_type: Option<&'a [u8]>,
}

impl CpuNode<'_> {
    /// Usable: `status` absent or "okay"/"ok".
    pub fn is_available(&self) -> bool {
        matches!(self.status, None | Some(b"okay") | Some(b"ok"))
    }
}

/// A string property without its terminating NUL.
fn trim_nul(v: &[u8]) -> &[u8] {
    v.split(|&c| c == 0).next().unwrap_or(v)
}

/// Big-endian u64 at `off` in `b`.
fn be64(b: &[u8], off: usize) -> Option<u64> {
    Some((u64::from(be32(b, off)?) << 32) | u64::from(be32(b, off.checked_add(4)?)?))
}

/// Each (address, size) pair of a `reg` value with `cells` = (address
/// cells, size cells), to `f`. Zero-sized entries are skipped.
fn reg_ranges(reg: &[u8], cells: (usize, usize), f: &mut impl FnMut(u64, u64)) {
    let (ac, sc) = cells;
    let entry = (ac + sc) * 4;
    for e in reg.chunks_exact(entry) {
        let (Some(base), Some(size)) = (cells_u64(&e[..ac * 4]), cells_u64(&e[ac * 4..])) else {
            return;
        };
        if size != 0 {
            f(base, size);
        }
    }
}

/// Big-endian cell value of 1 or 2 cells.
fn cells_u64(v: &[u8]) -> Option<u64> {
    match v.len() {
        4 => Some(u64::from(be32(v, 0)?)),
        8 => Some((u64::from(be32(v, 0)?) << 32) | u64::from(be32(v, 4)?)),
        _ => None,
    }
}

impl<'a> Fdt<'a> {
    /// Call `f` for every direct child of `/cpus` whose base name is `cpu`,
    /// in blob order. Nodes nested inside a CPU node (interrupt
    /// controllers, caches) are skipped. Stops quietly at malformed data.
    pub fn cpus(&self, mut f: impl FnMut(CpuNode<'a>)) {
        let b = self.structs;
        let mut off = 0usize;
        let mut depth = 0usize;
        let mut in_cpus = false; // inside /cpus (depth 2)
        let mut cur: Option<CpuNode<'a>> = None; // inside /cpus/cpu@N (depth 3)
        let mut step = || -> Option<bool> {
            let token = be32(b, off)?;
            off = off.checked_add(4)?;
            match token {
                FDT_BEGIN_NODE => {
                    let name = cstr(b, off)?;
                    off = align4(off.checked_add(name.len() + 1)?)?;
                    depth += 1;
                    let base = name.split(|&c| c == b'@').next().unwrap_or(name);
                    if depth == 2 && name == b"cpus" {
                        in_cpus = true;
                    } else if depth == 3 && in_cpus && base == b"cpu" {
                        cur = Some(CpuNode::default());
                    }
                }
                FDT_END_NODE => {
                    if depth == 0 {
                        return None;
                    }
                    if depth == 3 {
                        if let Some(node) = cur.take() {
                            f(node);
                        }
                    } else if depth == 2 && in_cpus {
                        return Some(false); // /cpus is closed: done
                    }
                    depth -= 1;
                }
                FDT_PROP => {
                    let len = be32(b, off)? as usize;
                    let nameoff = be32(b, off.checked_add(4)?)? as usize;
                    let val_off = off.checked_add(8)?;
                    let value = b.get(val_off..val_off.checked_add(len)?)?;
                    off = align4(val_off.checked_add(len)?)?;
                    if depth == 3 {
                        if let Some(node) = cur.as_mut() {
                            match cstr(self.strings, nameoff)? {
                                b"reg" => node.reg = cells_u64(value),
                                b"status" => node.status = Some(trim_nul(value)),
                                b"enable-method" => node.enable_method = Some(trim_nul(value)),
                                b"mmu-type" => node.mmu_type = Some(trim_nul(value)),
                                _ => {}
                            }
                        }
                    }
                }
                FDT_NOP => {}
                _ => return None,
            }
            Some(true)
        };
        while let Some(true) = step() {}
    }
}

/// Whether a RISC-V ISA description lists extension `ext` (ASCII,
/// case-insensitive): a `riscv,isa` string ("rv64imafdc_zicsr_sstc") or a
/// `riscv,isa-extensions` NUL-separated string list.
pub fn isa_has_extension(isa: &[u8], ext: &[u8]) -> bool {
    isa.split(|&c| c == b'_' || c == 0)
        .any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether a RISC-V ISA description lists single-letter extension `letter`
/// (ASCII, case-insensitive): in a `riscv,isa` string the letters after the
/// `rv32`/`rv64` prefix of the first `_`-separated token (`g` stands for
/// `imafd`); in a `riscv,isa-extensions` list an entry of that letter alone.
pub fn isa_has_base_extension(isa: &[u8], letter: u8) -> bool {
    let letter = letter.to_ascii_lowercase();
    let mut tokens = isa
        .split(|&c| c == b'_' || c == 0)
        .filter(|t| !t.is_empty());
    let Some(first) = tokens.next() else {
        return false;
    };
    // No allocation: this runs before the heap exists (RISC-V early boot).
    let is_prefix = first.len() >= 4
        && first[..2].eq_ignore_ascii_case(b"rv")
        && (&first[2..4] == b"32" || &first[2..4] == b"64");
    if is_prefix {
        let has = |l: u8| first[4..].iter().any(|c| c.to_ascii_lowercase() == l);
        return has(letter) || (has(b'g') && b"imafd".contains(&letter));
    }
    // An isa-extensions list: each entry is one extension.
    core::iter::once(first)
        .chain(tokens)
        .any(|t| t.len() == 1 && t[0].to_ascii_lowercase() == letter)
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
    fn finds_single_letter_extensions() {
        assert!(isa_has_base_extension(b"rv64imafdch_zicsr_sstc\0", b'd'));
        assert!(isa_has_base_extension(b"RV64IMAFDC\0", b'f'));
        assert!(isa_has_base_extension(b"rv64gc\0", b'd'), "g implies imafd");
        assert!(!isa_has_base_extension(b"rv64imac_zicsr\0", b'd'));
        // A multi-letter extension containing the letter is not it.
        assert!(!isa_has_base_extension(b"rv64imac_zdinx\0", b'd'));
        assert!(isa_has_base_extension(b"i\0m\0a\0f\0d\0c\0", b'd'));
        assert!(!isa_has_base_extension(b"i\0m\0a\0c\0zicsr\0", b'd'));
        assert!(!isa_has_base_extension(b"", b'd'));
    }

    /// / { #address-cells = <2>; #size-cells = <2>;
    ///     memory@40000000 { device_type = "memory";
    ///         reg = <0 0x40000000 0 0x10000000>, <0 0x80000000 0 0x1000>; };
    ///     ram { device_type = "memory"; reg = <1 0 0 0x2000>; };
    ///     memoryless@1 { reg = <0 1 0 1>; };
    ///     reserved-memory { #address-cells = <2>; #size-cells = <1>;
    ///         fw@40000000 { reg = <0 0x40000000 0x200000>; };
    ///         pool { size = <0x100000>; }; }; }
    /// with /memreserve/ 0x48000000 0x1000.
    fn memory_sample() -> Vec<u8> {
        let mut strings = Vec::new();
        let mut add = |n: &[u8]| {
            let off = strings.len();
            strings.extend_from_slice(n);
            strings.push(0);
            off
        };
        let (ac, sc, reg, dt, size) = (
            add(b"#address-cells"),
            add(b"#size-cells"),
            add(b"reg"),
            add(b"device_type"),
            add(b"size"),
        );
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
        let cells = |v: &[u32]| v.iter().flat_map(|c| c.to_be_bytes()).collect::<Vec<u8>>();
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"");
        prop(&mut st, ac, &cells(&[2]));
        prop(&mut st, sc, &cells(&[2]));
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"memory@40000000");
        prop(&mut st, dt, b"memory\0");
        prop(
            &mut st,
            reg,
            &cells(&[0, 0x4000_0000, 0, 0x1000_0000, 0, 0x8000_0000, 0, 0x1000]),
        );
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"ram");
        prop(&mut st, reg, &cells(&[1, 0, 0, 0x2000]));
        prop(&mut st, dt, b"memory\0");
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"memoryless@1");
        prop(&mut st, reg, &cells(&[0, 1, 0, 1]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"reserved-memory");
        prop(&mut st, ac, &cells(&[2]));
        prop(&mut st, sc, &cells(&[1]));
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"fw@40000000");
        prop(&mut st, reg, &cells(&[0, 0x4000_0000, 0x20_0000]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"pool");
        prop(&mut st, size, &cells(&[0x10_0000]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END);

        let mut rsv = Vec::new();
        for v in [0x4800_0000u64, 0x1000, 0, 0] {
            rsv.extend_from_slice(&v.to_be_bytes());
        }
        let off_rsv = 40usize;
        let off_struct = off_rsv + rsv.len();
        let off_strings = off_struct + st.len();
        let total = off_strings + strings.len();
        let mut b = Vec::new();
        for v in [
            FDT_MAGIC,
            total as u32,
            off_struct as u32,
            off_strings as u32,
            off_rsv as u32,
            17,
            16,
            0,
            strings.len() as u32,
            st.len() as u32,
        ] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&rsv);
        b.extend_from_slice(&st);
        b.extend_from_slice(&strings);
        b
    }

    /// / { #address-cells = <2>; #size-cells = <2>;
    ///     pl031@9010000 { compatible = "arm,pl031", "arm,primecell";
    ///         reg = <0 0x9010000 0 0x1000>; };
    ///     soc { #address-cells = <2>; #size-cells = <2>; ranges;
    ///         rtc@200 { compatible = "google,goldfish-rtc"; status =
    /// "disabled";             reg = <0 0x200 0 0x1000>; };
    ///         rtc@101000 { compatible = "google,goldfish-rtc";
    ///             reg = <0 0x101000 0 0x1000>; }; };
    ///     bus { #address-cells = <1>; #size-cells = <1>; ranges = <0
    /// 0x10000000 0x1000>;         dev@0 { compatible = "test,translated";
    /// reg = <0 0x100>; }; }; }
    fn device_sample() -> Vec<u8> {
        let mut strings = Vec::new();
        let mut add = |n: &[u8]| {
            let off = strings.len();
            strings.extend_from_slice(n);
            strings.push(0);
            off
        };
        let (ac, sc, reg, compat, status, ranges) = (
            add(b"#address-cells"),
            add(b"#size-cells"),
            add(b"reg"),
            add(b"compatible"),
            add(b"status"),
            add(b"ranges"),
        );
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
        let cells = |v: &[u32]| v.iter().flat_map(|c| c.to_be_bytes()).collect::<Vec<u8>>();
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"");
        prop(&mut st, ac, &cells(&[2]));
        prop(&mut st, sc, &cells(&[2]));
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"pl031@9010000");
        prop(&mut st, compat, b"arm,pl031\0arm,primecell\0");
        prop(&mut st, reg, &cells(&[0, 0x901_0000, 0, 0x1000]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"soc");
        prop(&mut st, ac, &cells(&[2]));
        prop(&mut st, sc, &cells(&[2]));
        prop(&mut st, ranges, b"");
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"rtc@200");
        prop(&mut st, compat, b"google,goldfish-rtc\0");
        prop(&mut st, status, b"disabled\0");
        prop(&mut st, reg, &cells(&[0, 0x200, 0, 0x1000]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"rtc@101000");
        prop(&mut st, compat, b"google,goldfish-rtc\0");
        prop(&mut st, reg, &cells(&[0, 0x10_1000, 0, 0x1000]));
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_END_NODE);
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"bus");
        prop(&mut st, ac, &cells(&[1]));
        prop(&mut st, sc, &cells(&[1]));
        prop(&mut st, ranges, &cells(&[0, 0x1000_0000, 0x1000]));
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"dev@0");
        prop(&mut st, compat, b"test,translated\0");
        prop(&mut st, reg, &cells(&[0, 0x100]));
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
    fn finds_devices_by_compatible() {
        let blob = device_sample();
        let fdt = Fdt::new(&blob).unwrap();
        // Any entry of the list matches, not only the first.
        assert_eq!(fdt.compatible_reg(b"arm,pl031"), Some((0x901_0000, 0x1000)));
        assert_eq!(
            fdt.compatible_reg(b"arm,primecell"),
            Some((0x901_0000, 0x1000))
        );
        // Disabled nodes are skipped; an empty `ranges` is the identity.
        assert_eq!(
            fdt.compatible_reg(b"google,goldfish-rtc"),
            Some((0x10_1000, 0x1000))
        );
        // A translating bus is not followed; prefixes do not match.
        assert_eq!(fdt.compatible_reg(b"test,translated"), None);
        assert_eq!(fdt.compatible_reg(b"arm,pl03"), None);
        assert_eq!(fdt.compatible_reg(b"missing"), None);
    }

    #[test]
    fn reads_memory_and_reserved_ranges() {
        let blob = memory_sample();
        let fdt = Fdt::new(&blob).unwrap();
        assert_eq!(fdt.size(), blob.len());
        let mut mem = Vec::new();
        fdt.memory_ranges(|b, s| mem.push((b, s)));
        assert_eq!(
            mem,
            [
                (0x4000_0000, 0x1000_0000),
                (0x8000_0000, 0x1000),
                (0x1_0000_0000, 0x2000)
            ]
        );
        let mut rsv = Vec::new();
        fdt.reserved_ranges(|b, s| rsv.push((b, s)));
        assert_eq!(rsv, [(0x4800_0000, 0x1000), (0x4000_0000, 0x20_0000)]);
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
    fn enumerates_cpus() {
        // The sample has one cpu@0 without reg; check that, then a richer
        // tree built the same way.
        let blob = sample();
        let fdt = Fdt::new(&blob).unwrap();
        let mut n = 0;
        fdt.cpus(|c| {
            n += 1;
            assert_eq!(c.reg, None);
            assert!(c.is_available());
        });
        assert_eq!(n, 1);

        let mut strings = Vec::new();
        let reg = strings.len();
        strings.extend_from_slice(b"reg\0");
        let status = strings.len();
        strings.extend_from_slice(b"status\0");
        let method = strings.len();
        strings.extend_from_slice(b"enable-method\0");
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
        name(&mut st, b"cpus");
        for (i, (r, stat)) in [(2u32, &b"okay\0"[..]), (5, b"disabled\0"), (7, b"")]
            .iter()
            .enumerate()
        {
            tok(&mut st, FDT_BEGIN_NODE);
            name(&mut st, alloc::format!("cpu@{i}").as_bytes());
            prop(&mut st, reg, &r.to_be_bytes());
            if !stat.is_empty() {
                prop(&mut st, status, stat);
            }
            prop(&mut st, method, b"psci\0");
            // A nested node with its own reg must not be taken for a CPU.
            tok(&mut st, FDT_BEGIN_NODE);
            name(&mut st, b"interrupt-controller");
            prop(&mut st, reg, &99u32.to_be_bytes());
            tok(&mut st, FDT_END_NODE);
            tok(&mut st, FDT_END_NODE);
        }
        tok(&mut st, FDT_BEGIN_NODE);
        name(&mut st, b"cpu-map");
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

        let fdt = Fdt::new(&b).unwrap();
        let mut got = Vec::new();
        fdt.cpus(|c| got.push((c.reg, c.is_available(), c.enable_method)));
        assert_eq!(
            got,
            [
                (Some(2), true, Some(&b"psci"[..])),
                (Some(5), false, Some(&b"psci"[..])),
                (Some(7), true, Some(&b"psci"[..])),
            ]
        );
        // Truncated anywhere: never panics.
        for cut in 0..b.len() {
            if let Some(fdt) = Fdt::new(&b[..cut]) {
                fdt.cpus(|_| {});
            }
        }
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
