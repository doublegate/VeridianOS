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

/// Where an ET_DYN program goes: Linux's `ELF_ET_DYN_BASE` on x86_64,
/// without randomisation.
pub const PIE_BASE: u64 = 0x5555_5555_4000;

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

/// Plan the image of `binary` (a file of `file_len` bytes). An ET_DYN file
/// is placed with its first page at `base`; an ET_EXEC file at its link
/// addresses (`base` unused). The image must lie in user space and the
/// entry point on an executable page.
pub fn plan(binary: &ElfBinary, file_len: usize, base: u64) -> Result<ImagePlan, ElfError> {
    let (start, end) = span(binary, file_len)?;
    let bias = if is_relocatable(binary) {
        base.wrapping_sub(start)
    } else {
        0
    };
    let size = end - start;
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
pub fn load(
    vas: &crate::mm::VirtualAddressSpace,
    program: (&ElfBinary, &dyn crate::fs::VfsNode, &[u8], &ImagePlan),
    interpreter: Option<(&ElfBinary, &dyn crate::fs::VfsNode, &[u8])>,
) -> Result<Loaded, crate::error::KernelError> {
    let (binary, node, data, program_plan) = program;
    map_image(vas, node, data, program_plan)?;
    let (start, base) = match interpreter {
        Some((interp, interp_node, interp_data)) => {
            let bad = |_| crate::error::KernelError::InvalidArgument {
                name: "interpreter",
                value: "not a loadable ELF image",
            };
            let size = check_interpreter(interp, interp_data.len()).map_err(bad)?;
            let at = vas.reserve_mmap_area(size as usize)?;
            let interp_plan = plan(interp, interp_data.len(), at.as_u64()).map_err(bad)?;
            map_image(vas, interp_node, interp_data, &interp_plan)?;
            (interp_plan.entry, interp_plan.bias)
        }
        None => (program_plan.entry, 0),
    };
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
/// writable segment); the other pages are written from `data`.
pub fn map_image(
    vas: &crate::mm::VirtualAddressSpace,
    node: &dyn crate::fs::VfsNode,
    data: &[u8],
    plan: &ImagePlan,
) -> Result<(), crate::error::KernelError> {
    use crate::mm::{vas::MappingType, VirtualAddress};
    let cache = node.page_cache();
    let mut first = 0;
    for run in plan.runs() {
        // PF_R/W/X to PROT_READ (1) / PROT_WRITE (2) / PROT_EXEC (4).
        let prot = ((run.flags & PF_R != 0) as usize)
            | (((run.flags & PF_W != 0) as usize) << 1)
            | (((run.flags & PF_X != 0) as usize) << 2);
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::elf::ElfSegment;

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
        let top = crate::mm::user_layout::USER_SPACE_END as u64;
        assert!(plan(&b, 0x10, top - 0x1000).is_ok());
        assert!(plan(&b, 0x10, top).is_err());
        assert!(plan(&b, 0x10, 0x5555_0000_0800).is_err()); // not page aligned
    }
}
