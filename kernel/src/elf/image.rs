//! Program image layout for exec (ADR 0010).
//!
//! [`plan`] turns a parsed ELF file into a page-by-page plan of what exec
//! maps: where each page goes once the load bias is applied, its
//! permissions, and where its contents come from. Planning is pure and
//! validates every PT_LOAD header first, so a malformed file is an error
//! and never a kernel panic; the mapping step only follows the plan.
//!
//! A page's contents are one of:
//! - [`PageSource::File`]: a whole page of the file, as Linux maps it. These
//!   pages can be shared between processes (the file's page cache).
//! - [`PageSource::Pieces`]: assembled from parts of the file, zero elsewhere:
//!   the page where a segment's file data ends and its BSS begins, or a page
//!   two segments share.
//! - [`PageSource::Zero`]: BSS only.

use alloc::{collections::BTreeMap, vec::Vec};

use super::{ElfBinary, ElfError, ElfType, SegmentType};

/// Segment permission bits (`p_flags`).
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// Where an ET_DYN program goes, without randomisation
/// (`mm::user_layout::PIE_BASE`: Linux's `ELF_ET_DYN_BASE` for each
/// architecture's layout).
pub const PIE_BASE: u64 = crate::mm::user_layout::PIE_BASE as u64;

const PAGE: u64 = 4096;

/// Part of a page that comes from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    /// Offset of the first byte within the page.
    pub page_offset: usize,
    /// Offset of the first byte within the file.
    pub file_offset: u64,
    pub len: usize,
}

/// Contents of one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSource {
    /// All zero.
    Zero,
    /// The whole page of the file at this (page-aligned) offset; bytes past
    /// the end of the file are zero.
    File(u64),
    /// These parts of the file, zero elsewhere.
    Pieces(Vec<Piece>),
}

/// One page of the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePlan {
    /// Address once loaded (bias applied).
    pub vaddr: u64,
    /// `PF_R | PF_W | PF_X` of every segment on the page.
    pub flags: u32,
    pub source: PageSource,
}

/// Consecutive pages with the same permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRun {
    pub start: u64,
    pub pages: usize,
    pub flags: u32,
}

/// What exec maps for one ELF file.
#[derive(Debug)]
pub struct ImagePlan {
    /// Added to every address of the file (0 for ET_EXEC).
    pub bias: u64,
    /// Entry point, biased.
    pub entry: u64,
    /// Address of the program header table once loaded (AT_PHDR), if a
    /// loaded segment holds it.
    pub phdr: Option<u64>,
    /// Every page, ascending.
    pub pages: Vec<PagePlan>,
}

impl ImagePlan {
    /// First page.
    pub fn start(&self) -> u64 {
        self.pages.first().map_or(0, |p| p.vaddr)
    }

    /// End of the last page.
    pub fn end(&self) -> u64 {
        self.pages.last().map_or(0, |p| p.vaddr + PAGE)
    }

    /// The pages grouped into runs of equal permissions.
    pub fn runs(&self) -> Vec<PageRun> {
        let mut runs: Vec<PageRun> = Vec::new();
        for page in &self.pages {
            match runs.last_mut() {
                Some(run)
                    if run.flags == page.flags
                        && run.start + run.pages as u64 * PAGE == page.vaddr =>
                {
                    run.pages += 1
                }
                _ => runs.push(PageRun {
                    start: page.vaddr,
                    pages: 1,
                    flags: page.flags,
                }),
            }
        }
        runs
    }
}

/// Whether the file is position-independent (ET_DYN): loaded at a base the
/// kernel chooses rather than at its link addresses.
pub fn is_relocatable(binary: &ElfBinary) -> bool {
    binary.elf_type == ElfType::SharedObject as u16
}

fn loads(binary: &ElfBinary) -> impl Iterator<Item = &super::ElfSegment> {
    binary
        .segments
        .iter()
        .filter(|s| s.segment_type == SegmentType::Load && s.memory_size > 0)
}

/// The page-aligned span `[start, end)` of the PT_LOAD segments as linked,
/// after checking each one against a file of `file_len` bytes.
pub fn span(binary: &ElfBinary, file_len: usize) -> Result<(u64, u64), ElfError> {
    let mut start = u64::MAX;
    let mut end = 0u64;
    for s in loads(binary) {
        let file_end = s.file_offset.checked_add(s.file_size);
        let mem_end = s.virtual_addr.checked_add(s.memory_size);
        let valid = s.file_size <= s.memory_size
            && file_end.is_some_and(|e| e <= file_len as u64)
            && s.virtual_addr % PAGE == s.file_offset % PAGE;
        let mem_end = match (
            valid,
            mem_end.and_then(|e| e.checked_next_multiple_of(PAGE)),
        ) {
            (true, Some(e)) => e,
            _ => return Err(ElfError::InvalidProgramHeader),
        };
        start = start.min(s.virtual_addr & !(PAGE - 1));
        end = end.max(mem_end);
    }
    if start == u64::MAX {
        return Err(ElfError::InvalidProgramHeader);
    }
    Ok((start, end))
}

/// The largest image `plan` accepts: well above any program here (the
/// largest, kwin_wayland, maps 66 MiB), and refused with ENOMEM above it.
pub const MAX_IMAGE_BYTES: u64 = 1 << 30;

/// Plan the image of `binary` (a file of `file_len` bytes). An ET_DYN file
/// is placed with its first page at `base`; an ET_EXEC file at its link
/// addresses (`base` unused). The image must lie in user space and the
/// entry point on an executable page.
pub fn plan(binary: &ElfBinary, file_len: usize, base: u64) -> Result<ImagePlan, ElfError> {
    // Another architecture's program (the parser accepts all three).
    if binary.machine != super::types::NATIVE_MACHINE {
        return Err(ElfError::UnsupportedMachine);
    }
    let (start, end) = span(binary, file_len)?;
    let bias = if is_relocatable(binary) {
        base.wrapping_sub(start)
    } else {
        0
    };
    let size = end - start;
    // Every page of the image is planned and given a frame now (BSS is not
    // mapped lazily yet), so an image larger than any real program -- a
    // crafted p_memsz near the size of user space -- is refused before
    // that: the kernel heap running out is fatal (PR #25 review).
    if size > MAX_IMAGE_BYTES {
        return Err(ElfError::MemoryAllocationFailed);
    }
    let loaded = start.wrapping_add(bias);
    if loaded % PAGE != 0 || !crate::mm::user_layout::is_user_range(loaded as usize, size as usize)
    {
        return Err(ElfError::InvalidProgramHeader);
    }

    // Per page (as linked): permissions, the exact file parts, and the
    // whole file page if the page can be one.
    struct Acc {
        flags: u32,
        pieces: Vec<Piece>,
        file_page: Option<Option<u64>>,
    }
    let mut acc: BTreeMap<u64, Acc> = BTreeMap::new();
    for s in loads(binary) {
        let flags = s.flags & (PF_R | PF_W | PF_X);
        let file_end = s.virtual_addr + s.file_size;
        let mem_end = s.virtual_addr + s.memory_size;
        let first = s.virtual_addr & !(PAGE - 1);
        let mut page = first;
        while page < mem_end {
            let lo = page.max(s.virtual_addr);
            let hi = (page + PAGE).min(file_end);
            let piece = (lo < hi).then(|| Piece {
                page_offset: (lo - page) as usize,
                file_offset: s.file_offset + (lo - s.virtual_addr),
                len: (hi - lo) as usize,
            });
            // Linux maps every page holding file data as the whole file
            // page, then zeroes after the file data where BSS follows.
            let bss_follows = s.memory_size > s.file_size && file_end < page + PAGE;
            // (Offset and address are congruent modulo the page size.)
            let file_page = (page < file_end && !bss_follows)
                .then(|| (s.file_offset & !(PAGE - 1)) + (page - first));
            let entry = acc.entry(page).or_insert(Acc {
                flags: 0,
                pieces: Vec::new(),
                file_page: None,
            });
            entry.flags |= flags;
            entry.pieces.extend(piece);
            // Two segments on one page keep the whole file page only if
            // both would map the same one.
            entry.file_page = match entry.file_page {
                None => Some(file_page),
                Some(prev) if prev == file_page => Some(prev),
                Some(_) => Some(None),
            };
            page += PAGE;
        }
    }

    let pages: Vec<PagePlan> = acc
        .into_iter()
        .map(|(page, a)| PagePlan {
            vaddr: page.wrapping_add(bias),
            flags: a.flags,
            source: match a.file_page.flatten() {
                Some(off) => PageSource::File(off),
                None if a.pieces.is_empty() => PageSource::Zero,
                None => PageSource::Pieces(a.pieces),
            },
        })
        .collect();

    let entry = binary.entry_point.wrapping_add(bias);
    let entry_page = entry & !(PAGE - 1);
    let executable = pages
        .binary_search_by_key(&entry_page, |p| p.vaddr)
        .is_ok_and(|i| pages[i].flags & PF_X != 0);
    if !executable {
        return Err(ElfError::InvalidProgramHeader);
    }

    Ok(ImagePlan {
        bias,
        entry,
        phdr: binary.phdr_vaddr().map(|v| v.wrapping_add(bias)),
        pages,
    })
}

/// Check an interpreter before exec commits: the same validation as
/// [`plan`] (at a stand-in base). Returns the size of its image, the
/// mmap-area space [`load`] reserves for it.
pub fn check_interpreter(binary: &ElfBinary, file_len: usize) -> Result<u64, ElfError> {
    let (start, end) = span(binary, file_len)?;
    plan(binary, file_len, PIE_BASE)?;
    Ok(end - start)
}

/// A program loaded by [`load`].
#[derive(Debug, Clone, Copy)]
pub struct Loaded {
    /// Where execution starts: the interpreter's entry point if there is
    /// one, otherwise the program's.
    pub start: u64,
    /// What the auxiliary vector says about it.
    pub aux: super::dynamic::ProgramAux,
}

/// Map a program (planned with [`plan`] before exec committed) and, for a
/// dynamically linked one, its interpreter (checked with
/// [`check_interpreter`]) into `vas`: each given as its parsed form, the
/// file node it was read from (for the page cache) and the data read. The
/// interpreter goes in the mmap area, like a library the loader maps later.
/// Which images BTI guards, as (program, interpreter), decided from their
/// property notes before anything is replaced (an exec calls this ahead of
/// its point of no return: a malformed note is ENOEXEC with the caller
/// intact, as Linux's load_elf_binary parses the notes before
/// begin_new_exec; it used to kill the caller -- PR #20 review). BTI
/// guarded code (arm64 `arch_elf_adjust_prot`): the kernel guards the
/// program only when it has no interpreter, and the interpreter by its own
/// note; an interpreter guards what it loads itself. Nothing is guarded
/// where the CPU has no BTI (`bti`).
pub fn bti_guards(
    program: (&ElfBinary, &[u8]),
    interpreter: Option<(&ElfBinary, &[u8])>,
    bti: bool,
) -> Result<(bool, bool), ElfError> {
    if !bti {
        return Ok((false, false));
    }
    Ok(match interpreter {
        None => (wants_bti(program.0, program.1)?, false),
        Some((interp, data)) => (false, wants_bti(interp, data)?),
    })
}

/// With `private`, nothing comes from the page cache: every page is written
/// from the bytes exec read and checked (`data`, `interp_data`). An exec that
/// grants privilege (file capabilities, set-user-ID or set-group-ID) maps
/// privately, so the program that runs privileged is the one that held the
/// privilege when it was read, not what a later write put in the file
/// (PR #33 review; Linux refuses such writes with ETXTBSY, which VeridianOS
/// does not have yet).
pub fn load(
    vas: &crate::mm::VirtualAddressSpace,
    program: (&ElfBinary, &dyn crate::fs::VfsNode, &[u8], &ImagePlan),
    interpreter: Option<(&ElfBinary, &dyn crate::fs::VfsNode, &[u8])>,
    guards: (bool, bool),
    private: bool,
) -> Result<Loaded, crate::error::KernelError> {
    let (binary, node, data, program_plan) = program;
    let (program_guarded, interp_guarded) = guards;
    // W^X follows the new program's note (the interpreter's does not
    // count), and PR_SET_MDWE the old program's (ADR 0017).
    vas.set_wx_policy(vas.wx_policy().exec(wants_wx(binary, data)));
    map_image(vas, node, data, program_plan, program_guarded, private)?;
    let mut interpreter_plan = None;
    let (start, base) = match interpreter {
        Some((interp, interp_node, interp_data)) => {
            let bad = |_| crate::error::KernelError::InvalidArgument {
                name: "interpreter",
                value: "not a loadable ELF image",
            };
            let size = check_interpreter(interp, interp_data.len()).map_err(bad)?;
            let at = vas.reserve_mmap_area(size as usize)?;
            let interp_plan = plan(interp, interp_data.len(), at.as_u64()).map_err(bad)?;
            map_image(
                vas,
                interp_node,
                interp_data,
                &interp_plan,
                interp_guarded,
                private,
            )?;
            let entry = (interp_plan.entry, interp_plan.bias);
            interpreter_plan = Some(interp_plan);
            entry
        }
        None => (program_plan.entry, 0),
    };
    // Instruction fetch is not coherent with the kernel's stores on
    // AArch64 and RISC-V: the executable pages are cleaned, then every
    // CPU's instruction side made to see them (HA-25).
    for p in program_plan.pages.iter().chain(
        interpreter_plan
            .as_ref()
            .map_or(&[][..], |plan| &plan.pages[..]),
    ) {
        if p.flags & PF_X != 0 {
            vas.clean_code(p.vaddr, 4096);
        }
    }
    crate::mm::vas::sync_icache();
    Ok(Loaded {
        start,
        aux: super::dynamic::ProgramAux {
            phdr: program_plan.phdr,
            phent: binary.phentsize,
            phnum: binary.phnum,
            entry: program_plan.entry,
            base,
        },
    })
}

/// Map the pages of `plan` (an image of the file `node`, read as `data`)
/// into `vas`: one mapping per run of equal permissions. Whole-file pages
/// come from the file's page cache when it has one, so every process
/// running the program shares them (read-only, or copy-on-write in a
/// writable segment); the other pages are written from `data`, and so is
/// every page with `private` (see [`load`]). With `guarded`, executable
/// pages are BTI guarded (PROT_BTI).
pub fn map_image(
    vas: &crate::mm::VirtualAddressSpace,
    node: &dyn crate::fs::VfsNode,
    data: &[u8],
    plan: &ImagePlan,
    guarded: bool,
    private: bool,
) -> Result<(), crate::error::KernelError> {
    use crate::mm::{vas::MappingType, VirtualAddress};
    let cache = if private { None } else { node.page_cache() };
    let mut first = 0;
    for run in plan.runs() {
        // PF_R/W/X to PROT_READ (1) / PROT_WRITE (2) / PROT_EXEC (4), and
        // PROT_BTI (0x10) on guarded code.
        let exec = run.flags & PF_X != 0;
        let prot = ((run.flags & PF_R != 0) as usize)
            | (((run.flags & PF_W != 0) as usize) << 1)
            | ((exec as usize) << 2)
            | (((exec && guarded) as usize) << 4);
        // Under PR_SET_MDWE a writable and executable segment is refused,
        // as Linux's mmap of it is.
        if vas.wx_policy().refuses_segment(prot) {
            return Err(crate::error::KernelError::PermissionDenied {
                operation: "exec: writable and executable segment under MDWE",
            });
        }
        let kind = if run.flags & PF_X != 0 {
            MappingType::Code
        } else {
            MappingType::Data
        };
        vas.map_region_fixed(
            VirtualAddress(run.start),
            run.pages * PAGE as usize,
            kind,
            Some(crate::mm::vas::user_prot_flags(prot)),
        )?;
        let pages = &plan.pages[first..first + run.pages];
        first += run.pages;
        if let Some(cache) = &cache {
            let mut frames = Vec::with_capacity(pages.len());
            for page in pages {
                let frame = match page.source {
                    // Filled from the file, not from `data`: a write to the
                    // file since it was read must not reach the cache.
                    PageSource::File(offset) => Some(cache.get(offset / PAGE, |buf| {
                        node.read(offset as usize, buf).map(|_| ())
                    })),
                    _ => None,
                };
                match frame.transpose() {
                    Ok(frame) => frames.push(frame),
                    Err(e) => {
                        crate::mm::page_cache::release(frames.into_iter().flatten());
                        return Err(e);
                    }
                }
            }
            vas.install_file_pages(VirtualAddress(run.start), &frames)?;
        }
        // Fresh pages are zero; the rest is written through the physical
        // map, whatever the pages' permissions.
        for page in pages {
            match &page.source {
                PageSource::Zero => {}
                PageSource::File(_) if cache.is_some() => {}
                PageSource::File(offset) => {
                    let from = (*offset as usize).min(data.len());
                    let to = from.saturating_add(PAGE as usize).min(data.len());
                    super::write_to_user_pages(vas, page.vaddr, &data[from..to])?;
                }
                PageSource::Pieces(pieces) => {
                    for piece in pieces {
                        // Checked by `span`: every piece lies inside the file.
                        let from = piece.file_offset as usize;
                        let bytes = data.get(from..from + piece.len).ok_or(
                            crate::error::KernelError::InvalidArgument {
                                name: "elf",
                                value: "segment data outside the file",
                            },
                        )?;
                        super::write_to_user_pages(
                            vas,
                            page.vaddr + piece.page_offset as u64,
                            bytes,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// PT_GNU_PROPERTY: the program's GNU property note.
const PT_GNU_PROPERTY: u32 = 0x6474_e553;
/// The note type of GNU properties.
const NT_GNU_PROPERTY_TYPE_0: u32 = 5;
/// The arm64 feature property, and its BTI bit.
const GNU_PROPERTY_AARCH64_FEATURE_1_AND: u32 = 0xc000_0000;
const GNU_PROPERTY_AARCH64_FEATURE_1_BTI: u32 = 1;

/// Whether a GNU property note (the bytes of a PT_GNU_PROPERTY segment)
/// marks the program as built for BTI. As Linux's `parse_elf_properties`
/// and arm64 `arch_parse_elf_property`: one NT_GNU_PROPERTY_TYPE_0 note
/// named "GNU", its properties 8-byte aligned in ascending type order, the
/// arm64 feature word exactly 4 bytes; anything malformed is ENOEXEC.
pub fn note_wants_bti(note: &[u8]) -> Result<bool, ElfError> {
    let bad = ElfError::InvalidProgramHeader;
    // A little-endian word at `at` of `b`, if it is there.
    let word = |b: &[u8], at: usize| -> Option<u32> {
        let w = b.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes(<[u8; 4]>::try_from(w).ok()?))
    };
    // Header (namesz, descsz, type), then the name "GNU\0", then the
    // descriptor at offset 16, a multiple of 8 bytes.
    let (Some(namesz), Some(descsz), Some(kind)) = (word(note, 0), word(note, 4), word(note, 8))
    else {
        return Err(bad);
    };
    if kind != NT_GNU_PROPERTY_TYPE_0 || namesz != 4 || note.get(12..16) != Some(b"GNU\0") {
        return Err(bad);
    }
    let descsz = descsz as usize;
    if !descsz.is_multiple_of(8) {
        return Err(bad);
    }
    let desc = note.get(16..16 + descsz).ok_or(bad)?;
    let mut at = 0usize;
    let mut prev: Option<u32> = None;
    let mut bti = false;
    while at < desc.len() {
        let (Some(kind), Some(size)) = (word(desc, at), word(desc, at + 4)) else {
            return Err(bad);
        };
        if prev.is_some_and(|p| kind <= p) {
            return Err(bad);
        }
        prev = Some(kind);
        let data = at + 8;
        let end = data.checked_add(size as usize).ok_or(bad)?;
        if end > desc.len() {
            return Err(bad);
        }
        if kind == GNU_PROPERTY_AARCH64_FEATURE_1_AND {
            if size != 4 {
                return Err(bad);
            }
            bti = word(desc, data).ok_or(bad)? & GNU_PROPERTY_AARCH64_FEATURE_1_BTI != 0;
        }
        at = end.checked_next_multiple_of(8).ok_or(bad)?;
    }
    Ok(bti)
}

/// Whether `binary` (whose file is `data`) asks for BTI: its
/// PT_GNU_PROPERTY note, if it has one.
pub fn wants_bti(binary: &ElfBinary, data: &[u8]) -> Result<bool, ElfError> {
    let Some(seg) = binary
        .segments
        .iter()
        .find(|s| s.segment_type == SegmentType::Other(PT_GNU_PROPERTY))
    else {
        return Ok(false);
    };
    let start = usize::try_from(seg.file_offset).map_err(|_| ElfError::InvalidProgramHeader)?;
    let len = usize::try_from(seg.file_size).map_err(|_| ElfError::InvalidProgramHeader)?;
    let end = start
        .checked_add(len)
        .ok_or(ElfError::InvalidProgramHeader)?;
    note_wants_bti(data.get(start..end).ok_or(ElfError::InvalidProgramHeader)?)
}

/// The owner of VeridianOS notes, the feature-control note's type, and its
/// bits: the W^X opt-in (`<veridian/wxneeded.h>`) and Linux uname mode
/// (`<veridian/linux-uname.h>`, ADR 0021).
const VERIDIAN_NOTE_NAME: &[u8] = b"VeridianOS\0";
const NT_VERIDIAN_FEATURE_CTL: u32 = 1;
const VERIDIAN_FCTL_WXNEEDED: u32 = 1;
const VERIDIAN_FCTL_LINUX_UNAME: u32 = 2;
/// The GNU ABI tag (`.note.ABI-tag`, glibc programs): its first word is
/// the OS, 0 for Linux.
const GNU_NOTE_NAME: &[u8] = b"GNU\0";
const NT_GNU_ABI_TAG: u32 = 1;
const ELF_NOTE_OS_LINUX: u32 = 0;

/// What the notes of an image ask for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteFeatures {
    /// The bits of every VeridianOS feature-control note, combined.
    pub bits: u32,
    /// A GNU ABI tag naming Linux.
    pub gnu_linux: bool,
}

impl NoteFeatures {
    fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
            gnu_linux: self.gnu_linux || other.gnu_linux,
        }
    }
}

/// The features the notes of one PT_NOTE segment (`note`, aligned to
/// `align` bytes, 4 or 8) ask for. Reading stops at a malformed note, and
/// what was read before it counts: as Linux, which ignores what it does not
/// read, exec does not fail, and nothing is granted that no note asked for.
pub fn note_features(note: &[u8], align: usize) -> NoteFeatures {
    let word = |at: usize| -> Option<u32> {
        let w = note.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes(<[u8; 4]>::try_from(w).ok()?))
    };
    let mut found = NoteFeatures::default();
    let mut at = 0usize;
    while at < note.len() {
        let (Some(namesz), Some(descsz), Some(kind)) = (word(at), word(at + 4), word(at + 8))
        else {
            break;
        };
        let name_at = at + 12;
        let Some(desc_at) = name_at
            .checked_add(namesz as usize)
            .and_then(|e| e.checked_next_multiple_of(align))
        else {
            break;
        };
        let Some(next) = desc_at
            .checked_add(descsz as usize)
            .and_then(|e| e.checked_next_multiple_of(align))
        else {
            break;
        };
        let (Some(name), Some(desc)) = (
            note.get(name_at..name_at + namesz as usize),
            note.get(desc_at..desc_at + descsz as usize),
        ) else {
            break;
        };
        let first = desc
            .get(..4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
        if name == VERIDIAN_NOTE_NAME && kind == NT_VERIDIAN_FEATURE_CTL && desc.len() == 4 {
            found.bits |= first.unwrap_or(0);
        } else if name == GNU_NOTE_NAME
            && kind == NT_GNU_ABI_TAG
            && desc.len() >= 16
            && first == Some(ELF_NOTE_OS_LINUX)
        {
            found.gnu_linux = true;
        }
        at = next;
    }
    found
}

/// Whether the notes of one PT_NOTE segment opt in to writable and
/// executable memory (see [`note_features`]).
pub fn note_wants_wx(note: &[u8], align: usize) -> bool {
    note_features(note, align).bits & VERIDIAN_FCTL_WXNEEDED != 0
}

/// The features the notes of `binary` (whose file is `data`) ask for: its
/// PT_NOTE segments, combined.
pub fn image_features(binary: &ElfBinary, data: &[u8]) -> NoteFeatures {
    binary
        .segments
        .iter()
        .filter(|s| s.segment_type == SegmentType::Note)
        .filter_map(|s| {
            let start = usize::try_from(s.file_offset).ok()?;
            let len = usize::try_from(s.file_size).ok()?;
            let align = if s.alignment == 8 { 8 } else { 4 };
            Some(note_features(
                data.get(start..start.checked_add(len)?)?,
                align,
            ))
        })
        .fold(NoteFeatures::default(), NoteFeatures::union)
}

/// Whether `binary` (whose file is `data`) opts in to writable and
/// executable memory: a VeridianOS feature-control note in any PT_NOTE.
pub fn wants_wx(binary: &ElfBinary, data: &[u8]) -> bool {
    image_features(binary, data).bits & VERIDIAN_FCTL_WXNEEDED != 0
}

/// Whether `binary` (whose file is `data`) runs in Linux uname mode: a
/// feature-control note asking for it, or a GNU ABI tag naming Linux, as
/// glibc programs carry (ADR 0021).
pub fn wants_linux_uname(binary: &ElfBinary, data: &[u8]) -> bool {
    let n = image_features(binary, data);
    n.bits & VERIDIAN_FCTL_LINUX_UNAME != 0 || n.gnu_linux
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::elf::ElfSegment;

    /// A GNU property note holding `props` (type, data) in that order.
    fn property_note(props: &[(u32, &[u8])]) -> Vec<u8> {
        let mut desc = Vec::new();
        for (t, d) in props {
            desc.extend_from_slice(&t.to_le_bytes());
            desc.extend_from_slice(&(d.len() as u32).to_le_bytes());
            desc.extend_from_slice(d);
            while desc.len() % 8 != 0 {
                desc.push(0);
            }
        }
        let mut n = Vec::new();
        n.extend_from_slice(&4u32.to_le_bytes());
        n.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        n.extend_from_slice(&5u32.to_le_bytes());
        n.extend_from_slice(b"GNU\0");
        n.extend_from_slice(&desc);
        n
    }

    #[test]
    fn veridian_wx_note_is_the_opt_in() {
        // One note: namesz, descsz, type, name padded, desc padded.
        fn note(name: &[u8], kind: u32, desc: &[u8], align: usize) -> Vec<u8> {
            let mut n = Vec::new();
            n.extend_from_slice(&(name.len() as u32).to_le_bytes());
            n.extend_from_slice(&(desc.len() as u32).to_le_bytes());
            n.extend_from_slice(&kind.to_le_bytes());
            n.extend_from_slice(name);
            n.resize(n.len().next_multiple_of(align), 0);
            n.extend_from_slice(desc);
            n.resize(n.len().next_multiple_of(align), 0);
            n
        }
        let wx = note(b"VeridianOS\0", 1, &[1, 0, 0, 0], 4);
        assert!(note_wants_wx(&wx, 4));
        // As <veridian/wxneeded.h> lays it out (and the toolchain links it).
        assert_eq!(wx.len(), 0x1c);
        // The bit clear, another type, another owner, a short descriptor.
        assert!(!note_wants_wx(
            &note(b"VeridianOS\0", 1, &[2, 0, 0, 0], 4),
            4
        ));
        assert!(!note_wants_wx(
            &note(b"VeridianOS\0", 2, &[1, 0, 0, 0], 4),
            4
        ));
        assert!(!note_wants_wx(
            &note(b"VeridianOT\0", 1, &[1, 0, 0, 0], 4),
            4
        ));
        assert!(!note_wants_wx(&note(b"VeridianOS\0", 1, &[1, 0], 4), 4));
        // After another note in the same segment.
        let mut two = note(b"GNU\0", 3, &[0xab; 20], 4);
        two.extend_from_slice(&wx);
        assert!(note_wants_wx(&two, 4));
        // In an 8-aligned segment.
        assert!(note_wants_wx(
            &note(b"VeridianOS\0", 1, &[1, 0, 0, 0], 8),
            8
        ));
        // Malformed: no opt-in, never a panic.
        assert!(!note_wants_wx(&wx[..wx.len() - 1], 4));
        assert!(!note_wants_wx(&wx[..10], 4));
        let mut huge = wx.clone();
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!note_wants_wx(&huge, 4));
        huge[0..4].copy_from_slice(&11u32.to_le_bytes());
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!note_wants_wx(&huge, 4));
        assert!(!note_wants_wx(&[], 4));

        // Only PT_NOTE segments are read, within the file.
        let seg = |kind, off, size| ElfSegment {
            segment_type: kind,
            virtual_addr: 0,
            physical_addr: 0,
            file_offset: off,
            file_size: size,
            memory_size: size,
            flags: PF_R,
            alignment: 4,
        };
        let mut file = vec![0u8; 0x40];
        file.extend_from_slice(&wx);
        let mut b = binary(
            ElfType::Executable,
            0,
            vec![seg(SegmentType::Load, 0x40, 0x1c)],
        );
        assert!(!wants_wx(&b, &file));
        b.segments.push(seg(SegmentType::Note, 0x40, 0x1c));
        assert!(wants_wx(&b, &file));
        b.segments[1].file_size = 0x1000;
        assert!(!wants_wx(&b, &file), "a note past the end of the file");
    }

    /// One note: namesz, descsz, type, name padded, desc padded.
    fn one_note(name: &[u8], kind: u32, desc: &[u8], align: usize) -> Vec<u8> {
        let mut n = Vec::new();
        n.extend_from_slice(&(name.len() as u32).to_le_bytes());
        n.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        n.extend_from_slice(&kind.to_le_bytes());
        n.extend_from_slice(name);
        n.resize(n.len().next_multiple_of(align), 0);
        n.extend_from_slice(desc);
        n.resize(n.len().next_multiple_of(align), 0);
        n
    }

    /// Every VeridianOS feature-control note counts: their bits are
    /// combined, so a program carrying both <veridian/wxneeded.h> and
    /// <veridian/linux-uname.h> gets both (ADR 0021).
    #[test]
    fn feature_notes_are_combined() {
        let wx = one_note(b"VeridianOS\0", 1, &1u32.to_le_bytes(), 4);
        let uname = one_note(b"VeridianOS\0", 1, &2u32.to_le_bytes(), 4);
        let mut both = wx.clone();
        both.extend_from_slice(&uname);
        let n = note_features(&both, 4);
        assert_eq!(n.bits, 3);
        assert!(!n.gnu_linux);
        assert!(note_wants_wx(&both, 4));
        let mut both = uname.clone();
        both.extend_from_slice(&wx);
        assert!(note_wants_wx(&both, 4), "the order does not matter");
        assert_eq!(note_features(&uname, 4).bits, 2);
        assert!(!note_wants_wx(&uname, 4));
        // A malformed note after a good one: what was read before counts.
        let mut bad = uname.clone();
        bad.extend_from_slice(&[0xff; 6]);
        assert_eq!(note_features(&bad, 4).bits, 2);
    }

    /// A GNU ABI tag (NT_GNU_ABI_TAG, "GNU", os word 0) names Linux, as
    /// glibc programs carry; another os, type or size does not.
    #[test]
    fn gnu_abi_tag_names_linux() {
        let tag = |os: u32, kind: u32, words: usize| {
            let mut d = os.to_le_bytes().to_vec();
            for v in [3u32, 2, 0].iter().take(words - 1) {
                d.extend_from_slice(&v.to_le_bytes());
            }
            one_note(b"GNU\0", kind, &d, 4)
        };
        assert!(note_features(&tag(0, 1, 4), 4).gnu_linux);
        assert!(!note_features(&tag(3, 1, 4), 4).gnu_linux, "FreeBSD");
        assert!(!note_features(&tag(0, 3, 4), 4).gnu_linux, "a build id");
        assert!(!note_features(&tag(0, 1, 2), 4).gnu_linux, "too short");
        assert_eq!(note_features(&tag(0, 1, 4), 4).bits, 0);
        // With the uname note in the same segment.
        let mut both = tag(0, 1, 4);
        both.extend_from_slice(&one_note(b"VeridianOS\0", 1, &2u32.to_le_bytes(), 4));
        let n = note_features(&both, 4);
        assert!(n.gnu_linux);
        assert_eq!(n.bits, 2);
    }

    #[test]
    fn gnu_property_note_bti() {
        let feature = |bits: u32| (0xc000_0000u32, bits.to_le_bytes());
        let (t, bti) = feature(1 | 2); // BTI and PAC
        assert_eq!(note_wants_bti(&property_note(&[(t, &bti)])), Ok(true));
        let (t, pac) = feature(2);
        assert_eq!(note_wants_bti(&property_note(&[(t, &pac)])), Ok(false));
        // Another property first (x86 ISA needed, 0xc0008002) is skipped.
        let other = 7u32.to_le_bytes();
        let note = property_note(&[(0x1000, &other), (t, &bti)]);
        assert_eq!(note_wants_bti(&note), Ok(true));
        // No arm64 property: no BTI.
        assert_eq!(
            note_wants_bti(&property_note(&[(0x1000, &other)])),
            Ok(false)
        );

        let bad = Err(ElfError::InvalidProgramHeader);
        // Feature word not 4 bytes.
        assert_eq!(
            note_wants_bti(&property_note(&[(t, &[1, 0, 0, 0, 0, 0, 0, 0])])),
            bad
        );
        // Properties out of order.
        assert_eq!(
            note_wants_bti(&property_note(&[(t, &bti), (0x1000, &other)])),
            bad
        );
        // Wrong note type or name, truncated note, data past the end.
        let mut n = property_note(&[(t, &bti)]);
        n[8] = 1;
        assert_eq!(note_wants_bti(&n), bad);
        let mut n = property_note(&[(t, &bti)]);
        n[12] = b'X';
        assert_eq!(note_wants_bti(&n), bad);
        let n = property_note(&[(t, &bti)]);
        assert_eq!(note_wants_bti(&n[..n.len() - 4]), bad);
        assert_eq!(note_wants_bti(&n[..10]), bad);
        let mut n = property_note(&[(t, &bti)]);
        n[20] = 200; // pr_datasz far past the descriptor
        assert_eq!(note_wants_bti(&n), bad);
    }

    fn load(vaddr: u64, offset: u64, filesz: u64, memsz: u64, flags: u32) -> ElfSegment {
        ElfSegment {
            segment_type: SegmentType::Load,
            virtual_addr: vaddr,
            physical_addr: vaddr,
            file_offset: offset,
            file_size: filesz,
            memory_size: memsz,
            flags,
            alignment: PAGE,
        }
    }

    fn binary(elf_type: ElfType, entry: u64, segments: Vec<ElfSegment>) -> ElfBinary {
        ElfBinary {
            elf_type: elf_type as u16,
            machine: crate::elf::types::NATIVE_MACHINE,
            entry_point: entry,
            load_base: 0,
            load_size: 0,
            phoff: 64,
            phnum: segments.len() as u16,
            phentsize: 56,
            segments,
            interpreter: None,
            dynamic: false,
        }
    }

    /// An image larger than any program the kernel can load is refused
    /// before a page is planned: a crafted p_memsz near the size of user
    /// space built one plan entry per page and then a frame for each, and
    /// the kernel heap running out is fatal (PR #25 review).
    #[test]
    fn huge_images_are_refused_before_planning() {
        let b = binary(
            ElfType::Executable,
            0x40_0000,
            alloc::vec![load(0x40_0000, 0, 0x1000, 1 << 44, PF_R | PF_X | PF_W)],
        );
        assert!(matches!(
            plan(&b, 0x1000, 0),
            Err(ElfError::MemoryAllocationFailed)
        ));
        let fits = binary(
            ElfType::Executable,
            0x40_0000,
            alloc::vec![load(0x40_0000, 0, 0x1000, 64 << 20, PF_R | PF_X | PF_W)],
        );
        assert!(plan(&fits, 0x1000, 0).is_ok());
    }

    /// Another architecture's program is not planned (ENOEXEC at exec).
    #[test]
    fn foreign_machine_is_refused() {
        let foreign = if crate::elf::types::NATIVE_MACHINE == 62 {
            183
        } else {
            62
        };
        let mut b = binary(
            ElfType::Executable,
            0x40_0000,
            alloc::vec![load(0x40_0000, 0, 0x1000, 0x1000, PF_R | PF_X)],
        );
        assert!(plan(&b, 0x1000, PIE_BASE).is_ok());
        b.machine = foreign;
        assert!(matches!(
            plan(&b, 0x1000, PIE_BASE),
            Err(ElfError::UnsupportedMachine)
        ));
    }

    /// A typical static program: text (RX) from offset 0, then data (RW)
    /// with BSS, at its link addresses.
    #[test]
    fn static_program_maps_at_its_link_addresses() {
        let b = binary(
            ElfType::Executable,
            0x40_1000,
            vec![
                load(0x40_0000, 0, 0x2345, 0x2345, PF_R | PF_X),
                load(0x40_4345, 0x3345, 0x100, 0x3000, PF_R | PF_W),
            ],
        );
        let p = plan(&b, 0x3445, 0).unwrap();
        assert_eq!(p.bias, 0);
        assert_eq!(p.entry, 0x40_1000);
        assert_eq!(p.phdr, Some(0x40_0040));
        assert_eq!(p.start(), 0x40_0000);
        assert_eq!(p.end(), 0x40_8000);
        // Text: three whole file pages (the last one is the file page,
        // as Linux maps it, past the segment's end).
        assert_eq!(p.pages[0].source, PageSource::File(0));
        assert_eq!(p.pages[2].source, PageSource::File(0x2000));
        // Data: its file data ends inside the first page and BSS follows,
        // so that page is assembled; the rest is BSS.
        assert_eq!(p.pages[3].vaddr, 0x40_4000);
        assert_eq!(
            p.pages[3].source,
            PageSource::Pieces(vec![Piece {
                page_offset: 0x345,
                file_offset: 0x3345,
                len: 0x100,
            }])
        );
        assert_eq!(p.pages[4].source, PageSource::Zero);
        assert_eq!(
            p.runs(),
            vec![
                PageRun {
                    start: 0x40_0000,
                    pages: 3,
                    flags: PF_R | PF_X
                },
                PageRun {
                    start: 0x40_4000,
                    pages: 4,
                    flags: PF_R | PF_W
                },
            ]
        );
    }

    /// An ET_DYN file (PIE or ld.so) is linked at 0 and goes at `base`:
    /// every address moves with it.
    #[test]
    fn relocatable_image_moves_to_its_base() {
        let mut b = binary(
            ElfType::SharedObject,
            0x1100,
            vec![
                load(0, 0, 0x1200, 0x1200, PF_R | PF_X),
                load(0x2200, 0x1200, 0x10, 0x10, PF_R | PF_W),
            ],
        );
        b.segments.push(ElfSegment {
            segment_type: SegmentType::Phdr,
            ..load(0x40, 0x40, 0x70, 0x70, PF_R)
        });
        let p = plan(&b, 0x1210, PIE_BASE).unwrap();
        assert_eq!(p.bias, PIE_BASE);
        assert_eq!(p.entry, PIE_BASE + 0x1100);
        assert_eq!(p.phdr, Some(PIE_BASE + 0x40));
        assert_eq!(p.start(), PIE_BASE);
        assert_eq!(p.pages[0].source, PageSource::File(0));
        assert_eq!(p.pages[1].source, PageSource::File(0x1000));
        // The data page maps the file page holding offset 0x1200.
        assert_eq!(p.pages[2].vaddr, PIE_BASE + 0x2000);
        assert_eq!(p.pages[2].source, PageSource::File(0x1000));
    }

    /// Two segments on one page: the permissions of both, and the file
    /// data of both when they come from different file pages.
    #[test]
    fn a_shared_page_gets_both_segments() {
        let b = binary(
            ElfType::Executable,
            0x40_0000,
            vec![
                load(0x40_0000, 0, 0x800, 0x800, PF_R | PF_X),
                load(0x40_0800, 0x2800, 0x100, 0x100, PF_R | PF_W),
            ],
        );
        let p = plan(&b, 0x2900, 0).unwrap();
        assert_eq!(p.pages.len(), 1);
        assert_eq!(p.pages[0].flags, PF_R | PF_W | PF_X);
        assert_eq!(
            p.pages[0].source,
            PageSource::Pieces(vec![
                Piece {
                    page_offset: 0,
                    file_offset: 0,
                    len: 0x800
                },
                Piece {
                    page_offset: 0x800,
                    file_offset: 0x2800,
                    len: 0x100
                },
            ])
        );
        // The same file page for both keeps the whole page.
        let b = binary(
            ElfType::Executable,
            0x40_0000,
            vec![
                load(0x40_0000, 0, 0x800, 0x800, PF_R | PF_X),
                load(0x40_0800, 0x800, 0x100, 0x100, PF_R | PF_W),
            ],
        );
        assert_eq!(
            plan(&b, 0x900, 0).unwrap().pages[0].source,
            PageSource::File(0)
        );
    }

    /// Malformed headers are errors: nothing is sliced or added unchecked.
    #[test]
    fn malformed_segments_are_rejected() {
        let bad = |segs: Vec<ElfSegment>, file_len: usize| {
            let b = binary(ElfType::Executable, 0x40_0000, segs);
            plan(&b, file_len, 0).is_err()
        };
        // File data past the end of the file.
        assert!(bad(
            vec![load(0x40_0000, 0x1000, 0x2000, 0x2000, PF_R | PF_X)],
            0x2000
        ));
        // Offset + size overflows.
        assert!(bad(
            vec![load(0x40_0000, u64::MAX - 8, 0x100, 0x100, PF_R | PF_X)],
            0x1000
        ));
        // Address + size overflows.
        assert!(bad(
            vec![load(u64::MAX - 0xFFF, 0, 0x10, 0x2000, PF_R | PF_X)],
            0x1000
        ));
        // More file data than memory.
        assert!(bad(
            vec![load(0x40_0000, 0, 0x200, 0x100, PF_R | PF_X)],
            0x1000
        ));
        // Address and offset not congruent modulo the page size.
        assert!(bad(
            vec![load(0x40_0010, 0, 0x100, 0x100, PF_R | PF_X)],
            0x1000
        ));
        // No loadable segment.
        assert!(bad(vec![], 0x1000));
        // In the kernel half, or on the null page.
        assert!(bad(
            vec![load(0xFFFF_8000_0000_0000, 0, 0x100, 0x100, PF_R | PF_X)],
            0x1000
        ));
        assert!(bad(vec![load(0, 0, 0x100, 0x100, PF_R | PF_X)], 0x1000));
    }

    /// The entry point must be on an executable page of the image.
    #[test]
    fn entry_point_must_be_executable() {
        let segs = || {
            vec![
                load(0x40_0000, 0, 0x1000, 0x1000, PF_R | PF_X),
                load(0x40_1000, 0x1000, 0x10, 0x10, PF_R | PF_W),
            ]
        };
        assert!(plan(&binary(ElfType::Executable, 0x40_0800, segs()), 0x1010, 0).is_ok());
        assert!(plan(&binary(ElfType::Executable, 0x40_1000, segs()), 0x1010, 0).is_err());
        assert!(plan(&binary(ElfType::Executable, 0x50_0000, segs()), 0x1010, 0).is_err());
    }

    /// AT_PHDR: PT_PHDR's address if there is one, else the address of
    /// e_phoff inside the PT_LOAD that holds it, else none (a table outside
    /// every segment is not in memory).
    #[test]
    fn header_table_address() {
        let text = || load(0x40_0000, 0, 0x1000, 0x1000, PF_R | PF_X);
        let mut b = binary(ElfType::Executable, 0x40_0000, vec![text()]);
        assert_eq!(plan(&b, 0x1000, 0).unwrap().phdr, Some(0x40_0040));
        b.segments.push(ElfSegment {
            segment_type: SegmentType::Phdr,
            ..load(0x40_0100, 0x100, 0x1f8, 0x1f8, PF_R)
        });
        assert_eq!(plan(&b, 0x1000, 0).unwrap().phdr, Some(0x40_0100));
        let mut b = binary(
            ElfType::Executable,
            0x40_1000,
            vec![load(0x40_1000, 0x1000, 0x1000, 0x1000, PF_R | PF_X)],
        );
        b.phoff = 64;
        assert_eq!(plan(&b, 0x2000, 0).unwrap().phdr, None);
    }

    /// An ET_DYN image placed so it would leave user space is refused.
    #[test]
    fn relocated_image_must_stay_in_user_space() {
        let b = binary(
            ElfType::SharedObject,
            0,
            vec![load(0, 0, 0x10, 0x10, PF_R | PF_X)],
        );
        let top = crate::mm::user_layout::user_space_end() as u64;
        assert!(plan(&b, 0x10, top - 0x1000).is_ok());
        assert!(plan(&b, 0x10, top).is_err());
        assert!(plan(&b, 0x10, 0x5555_0000_0800).is_err()); // not page aligned
    }
}
