//! Virtual Filesystem (VFS) Layer
//!
//! Provides a unified interface for different filesystem implementations.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::sync::Arc;
use spin::RwLock;
use alloc::collections::BTreeMap;
use alloc::format;

pub mod ramfs;
pub mod devfs;
pub mod procfs;
pub mod file;

pub use file::{File, FileDescriptor, FileTable, OpenFlags, SeekFrom};

/// Maximum path length
pub const PATH_MAX: usize = 4096;

/// Maximum filename length
pub const NAME_MAX: usize = 255;

/// Filesystem node types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    File,
    Directory,
    CharDevice,
    BlockDevice,
    Pipe,
    Socket,
    Symlink,
}

/// File permissions (Unix-style)
#[derive(Debug, Clone, Copy)]
pub struct Permissions {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_exec: bool,
    pub group_read: bool,
    pub group_write: bool,
    pub group_exec: bool,
    pub other_read: bool,
    pub other_write: bool,
    pub other_exec: bool,
}

impl Permissions {
    /// Create default permissions (rwxr-xr-x)
    pub fn default() -> Self {
        Self {
            owner_read: true,
            owner_write: true,
            owner_exec: true,
            group_read: true,
            group_write: false,
            group_exec: true,
            other_read: true,
            other_write: false,
            other_exec: true,
        }
    }
    
    /// Create read-only permissions
    pub fn read_only() -> Self {
        Self {
            owner_read: true,
            owner_write: false,
            owner_exec: false,
            group_read: true,
            group_write: false,
            group_exec: false,
            other_read: true,
            other_write: false,
            other_exec: false,
        }
    }
    
    /// Create permissions from Unix mode bits
    pub fn from_mode(mode: u32) -> Self {
        Self {
            owner_read:  (mode & 0o400) != 0,
            owner_write: (mode & 0o200) != 0,
            owner_exec:  (mode & 0o100) != 0,
            group_read:  (mode & 0o040) != 0,
            group_write: (mode & 0o020) != 0,
            group_exec:  (mode & 0o010) != 0,
            other_read:  (mode & 0o004) != 0,
            other_write: (mode & 0o002) != 0,
            other_exec:  (mode & 0o001) != 0,
        }
    }
}

/// File metadata
#[derive(Debug, Clone)]
pub struct Metadata {
    pub node_type: NodeType,
    pub size: usize,
    pub permissions: Permissions,
    pub uid: u32,
    pub gid: u32,
    pub created: u64,
    pub modified: u64,
    pub accessed: u64,
}

/// Directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub node_type: NodeType,
    pub inode: u64,
}

/// VFS node operations trait
pub trait VfsNode: Send + Sync {
    /// Read data from the node
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str>;
    
    /// Write data to the node
    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, &'static str>;
    
    /// Get metadata for the node
    fn metadata(&self) -> Result<Metadata, &'static str>;
    
    /// List directory entries (if this is a directory)
    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str>;
    
    /// Look up a child node by name (if this is a directory)
    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str>;
    
    /// Create a new file in this directory
    fn create(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, &'static str>;
    
    /// Create a new directory in this directory
    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, &'static str>;
    
    /// Remove a file or empty directory
    fn unlink(&self, name: &str) -> Result<(), &'static str>;
    
    /// Truncate the file to the specified size
    fn truncate(&self, size: usize) -> Result<(), &'static str>;
}

/// Filesystem trait
pub trait Filesystem: Send + Sync {
    /// Get the root node of the filesystem
    fn root(&self) -> Arc<dyn VfsNode>;
    
    /// Get filesystem name
    fn name(&self) -> &str;
    
    /// Check if filesystem is read-only
    fn is_readonly(&self) -> bool;
    
    /// Sync filesystem to disk
    fn sync(&self) -> Result<(), &'static str>;
}

/// Mount point information
pub struct MountPoint {
    pub path: String,
    pub filesystem: Arc<dyn Filesystem>,
}

/// Virtual Filesystem Manager
pub struct Vfs {
    /// Root filesystem
    root_fs: Option<Arc<dyn Filesystem>>,
    
    /// Mount points
    mounts: BTreeMap<String, Arc<dyn Filesystem>>,
    
    /// Current working directory for processes
    /// TODO: Move this to per-process data
    cwd: String,
}

impl Vfs {
    /// Create a new VFS instance
    pub fn new() -> Self {
        Self {
            root_fs: None,
            mounts: BTreeMap::new(),
            cwd: String::from("/"),
        }
    }
    
    /// Mount the root filesystem
    pub fn mount_root(&mut self, fs: Arc<dyn Filesystem>) -> Result<(), &'static str> {
        if self.root_fs.is_some() {
            return Err("Root filesystem already mounted");
        }
        self.root_fs = Some(fs);
        Ok(())
    }
    
    /// Mount a filesystem at the specified path
    pub fn mount(&mut self, path: String, fs: Arc<dyn Filesystem>) -> Result<(), &'static str> {
        if self.root_fs.is_none() {
            return Err("Root filesystem not mounted");
        }
        
        if self.mounts.contains_key(&path) {
            return Err("Path already mounted");
        }
        
        self.mounts.insert(path, fs);
        Ok(())
    }
    
    /// Mount a filesystem by type at the specified path
    pub fn mount_by_type(&mut self, path: &str, fs_type: &str, _flags: u32) -> Result<(), &'static str> {
        let fs: Arc<dyn Filesystem> = match fs_type {
            "ramfs" => Arc::new(ramfs::RamFs::new()),
            "devfs" => Arc::new(devfs::DevFs::new()),
            "procfs" => Arc::new(procfs::ProcFs::new()),
            _ => return Err("Unknown filesystem type"),
        };
        
        if path == "/" {
            self.mount_root(fs)
        } else {
            self.mount(path.into(), fs)
        }
    }
    
    /// Unmount a filesystem at the specified path
    pub fn unmount(&mut self, path: &str) -> Result<(), &'static str> {
        self.mounts.remove(path)
            .ok_or("Path not mounted")
            .map(|_| ())
    }
    
    /// Resolve a path to a VFS node
    pub fn resolve_path(&self, path: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        let root_fs = self.root_fs.as_ref()
            .ok_or("Root filesystem not mounted")?;
        
        // Normalize path
        let path = if path.starts_with('/') {
            path.into()
        } else {
            // Relative path - prepend CWD
            format!("{}/{}", self.cwd, path)
        };
        
        // Check if path is under a mount point
        for (mount_path, fs) in self.mounts.iter().rev() {
            if path.starts_with(mount_path) {
                let relative_path = &path[mount_path.len()..];
                return self.traverse_path(fs.root(), relative_path);
            }
        }
        
        // Use root filesystem
        self.traverse_path(root_fs.root(), &path)
    }
    
    /// Traverse a path from a starting node
    fn traverse_path(&self, mut node: Arc<dyn VfsNode>, path: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        // Keep track of path components for parent traversal
        let mut path_stack: Vec<Arc<dyn VfsNode>> = Vec::new();
        path_stack.push(node.clone());
        
        let components: Vec<&str> = path.split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        
        for component in components {
            if component == ".." {
                // Go back to parent directory
                if path_stack.len() > 1 {
                    path_stack.pop();
                    node = path_stack.last().unwrap().clone();
                }
                // If at root, stay at root
            } else {
                // Move forward to child
                node = node.lookup(component)?;
                path_stack.push(node.clone());
            }
        }
        
        Ok(node)
    }
    
    /// Get current working directory
    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }
    
    /// Set current working directory
    pub fn set_cwd(&mut self, path: String) -> Result<(), &'static str> {
        // Verify the path exists and is a directory
        let node = self.resolve_path(&path)?;
        let metadata = node.metadata()?;
        
        if metadata.node_type != NodeType::Directory {
            return Err("Not a directory");
        }
        
        self.cwd = path;
        Ok(())
    }
    
    /// Open a file
    pub fn open(&self, path: &str, _flags: OpenFlags) -> Result<Arc<dyn VfsNode>, &'static str> {
        self.resolve_path(path)
    }
    
    /// Create a directory
    pub fn mkdir(&self, path: &str, permissions: Permissions) -> Result<(), &'static str> {
        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err("Invalid path");
        };
        
        // Get parent directory
        let parent = self.resolve_path(parent_path)?;
        
        // Create directory in parent
        parent.mkdir(name, permissions)?;
        Ok(())
    }
    
    /// Remove a file or directory
    pub fn unlink(&self, path: &str) -> Result<(), &'static str> {
        // Split path into parent and name
        let (parent_path, name) = if let Some(pos) = path.rfind('/') {
            if pos == 0 {
                ("/", &path[1..])
            } else {
                (&path[..pos], &path[pos + 1..])
            }
        } else {
            return Err("Invalid path");
        };
        
        // Get parent directory
        let parent = self.resolve_path(parent_path)?;
        
        // Remove from parent
        parent.unlink(name)
    }
    
    /// Sync all filesystems
    pub fn sync(&self) -> Result<(), &'static str> {
        // Sync root filesystem
        if let Some(ref root) = self.root_fs {
            root.sync()?;
        }
        
        // Sync all mounted filesystems
        for fs in self.mounts.values() {
            fs.sync()?;
        }
        
        Ok(())
    }
}

/// Global VFS instance - use static mut for x86_64 to avoid spin::Once deadlock
#[cfg(target_arch = "x86_64")]
static mut VFS_X86: Option<alloc::boxed::Box<RwLock<Vfs>>> = None;

/// Global VFS instance for AArch64/RISC-V (avoiding spin::Once issues)
/// IMPORTANT: Must be explicitly None - uninitialized memory is not guaranteed to be zero on AArch64
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static mut VFS_STATIC: Option<alloc::boxed::Box<RwLock<Vfs>>> = None;

/// Get the VFS instance (architecture-specific)
pub fn get_vfs() -> &'static RwLock<Vfs> {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            VFS_X86.as_ref().expect("VFS not initialized").as_ref()
        }
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        unsafe {
            VFS_STATIC.as_ref().expect("VFS not initialized").as_ref()
        }
    }
}

/// Initialize the VFS with a RAM filesystem as root
pub fn init() {
    use crate::println;
    
    // Direct UART output for AArch64 debugging
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[VFS-DIRECT] Starting VFS init function...\n");
        uart_write_str("[VFS] Initializing Virtual Filesystem...\n");
    }
    
    #[cfg(not(target_arch = "aarch64"))]
    println!("[VFS] Initializing Virtual Filesystem...");
    
    // Initialize the VFS
    #[cfg(target_arch = "x86_64")]
    {
        println!("[VFS] Initializing VFS for x86_64...");
        unsafe {
            if VFS_X86.is_some() {
                println!("[VFS] WARNING: VFS already initialized! Skipping re-initialization.");
                return;
            }
            
            println!("[VFS] Creating BTreeMap...");
            let mounts = BTreeMap::new();
            println!("[VFS] BTreeMap created successfully");
            
            println!("[VFS] Creating String from /...");
            let cwd = String::from("/");
            println!("[VFS] String created successfully");
            
            println!("[VFS] Creating Vfs struct...");
            let vfs = Vfs {
                root_fs: None,
                mounts,
                cwd,
            };
            println!("[VFS] Vfs struct created successfully");
            
            println!("[VFS] Creating RwLock...");
            let rwlock = RwLock::new(vfs);
            println!("[VFS] RwLock created successfully");
            
            println!("[VFS] Boxing the RwLock...");
            let boxed_vfs = alloc::boxed::Box::new(rwlock);
            println!("[VFS] Box created successfully");
            
            println!("[VFS] Storing in VFS_X86...");
            VFS_X86 = Some(boxed_vfs);
            println!("[VFS] VFS initialized successfully");
        }
    }
    
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        #[cfg(target_arch = "aarch64")]
        {
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] Initializing VFS (static path for AArch64)...\n");
                
                // Add memory barriers before static mut initialization
                uart_write_str("[VFS] Adding memory barriers for safe initialization...\n");
                core::arch::asm!(
                    "dsb sy",  // Data Synchronization Barrier - ensures all memory accesses complete
                    "isb",     // Instruction Synchronization Barrier - flushes pipeline
                    options(nostack, nomem)
                );
                
                // Now it should be safe to initialize
                VFS_STATIC = None;
                
                // Add another barrier after initialization
                core::arch::asm!(
                    "dsb sy",
                    "isb",
                    options(nostack, nomem)
                );
                
                uart_write_str("[VFS] Static initialization complete with memory barriers\n");
            }
        }
        #[cfg(target_arch = "riscv64")]
        {
            println!("[VFS] Initializing VFS (static path for RISC-V)...");
            unsafe {
                VFS_STATIC = None;  // Also force to None on RISC-V for safety
            }
        }
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Entering unsafe block for VFS init...\n");
        }
        
        unsafe {
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] Inside unsafe block, checking if VFS_STATIC is_some()...\n");
            }
            
            if VFS_STATIC.is_some() {
                #[cfg(target_arch = "aarch64")]
                {
                    use crate::arch::aarch64::direct_uart::uart_write_str;
                    uart_write_str("[VFS] WARNING: VFS already initialized! Skipping re-initialization.\n");
                }
                #[cfg(target_arch = "riscv64")]
                println!("[VFS] WARNING: VFS already initialized! Skipping re-initialization.");
                return;
            }
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] VFS_STATIC is None, proceeding with initialization...\n");
            }
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] About to create BTreeMap...\n");
            }
            #[cfg(target_arch = "riscv64")]
            println!("[VFS] About to create BTreeMap...");
            
            let mounts = BTreeMap::new();
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] BTreeMap created successfully\n");
                uart_write_str("[VFS] About to create String from '/'...\n");
            }
            #[cfg(target_arch = "riscv64")]
            {
                println!("[VFS] BTreeMap created successfully");
                println!("[VFS] About to create String from '/'...");
            }
            
            let cwd = String::from("/");
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] String created successfully\n");
                uart_write_str("[VFS] About to create Vfs struct...\n");
            }
            #[cfg(target_arch = "riscv64")]
            {
                println!("[VFS] String created successfully");
                println!("[VFS] About to create Vfs struct...");
            }
            
            let vfs = Vfs {
                root_fs: None,
                mounts,
                cwd,
            };
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] Vfs struct created successfully\n");
                uart_write_str("[VFS] About to create RwLock...\n");
            }
            #[cfg(target_arch = "riscv64")]
            {
                println!("[VFS] Vfs struct created successfully");
                println!("[VFS] About to create RwLock...");
            }
            
            let rwlock = RwLock::new(vfs);
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] RwLock created successfully\n");
                uart_write_str("[VFS] About to box the RwLock...\n");
            }
            #[cfg(target_arch = "riscv64")]
            {
                println!("[VFS] RwLock created successfully");
                println!("[VFS] About to box the RwLock...");
            }
            
            let boxed_vfs = alloc::boxed::Box::new(rwlock);
            
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] Box created successfully\n");
                uart_write_str("[VFS] About to store in VFS_STATIC...\n");
            }
            #[cfg(target_arch = "riscv64")]
            {
                println!("[VFS] Box created successfully");
                println!("[VFS] About to store in VFS_STATIC...");
            }
            
            // Apply memory barriers for AArch64 before critical assignment
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[VFS] Applying memory barriers before VFS_STATIC assignment...\n");
                core::arch::asm!(
                    "dsb sy",  // Data Synchronization Barrier
                    "isb",     // Instruction Synchronization Barrier  
                    options(nostack, nomem)
                );
            }
            
            VFS_STATIC = Some(boxed_vfs);
            
            // Apply memory barriers for AArch64 after assignment
            #[cfg(target_arch = "aarch64")]
            {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                core::arch::asm!(
                    "dsb sy",  // Ensure write is complete
                    "isb",     // Flush pipeline
                    options(nostack, nomem)
                );
                uart_write_str("[VFS] VFS_STATIC assigned successfully with memory barriers\n");
                uart_write_str("[VFS] VFS initialized\n");
            }
            #[cfg(target_arch = "riscv64")]
            println!("[VFS] VFS initialized");
        }
    }
    
    #[cfg(feature = "alloc")]
    {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Creating RAM filesystem...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Creating RAM filesystem...");
        
        // Create a RAM filesystem as the root
        let ramfs = ramfs::RamFs::new();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] RAM filesystem created\n");
            uart_write_str("[VFS] Getting root node...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            println!("[VFS] RAM filesystem created");
            println!("[VFS] Getting root node...");
        }
        
        // Create essential directories
        let root = ramfs.root();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Creating essential directories...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Creating essential directories...");
        
        root.mkdir("dev", Permissions::default()).ok();
        root.mkdir("proc", Permissions::default()).ok();
        root.mkdir("bin", Permissions::default()).ok();
        root.mkdir("etc", Permissions::default()).ok();
        root.mkdir("tmp", Permissions::default()).ok();
        root.mkdir("home", Permissions::default()).ok();
        root.mkdir("usr", Permissions::default()).ok();
        root.mkdir("lib", Permissions::default()).ok();
        root.mkdir("sbin", Permissions::default()).ok();
        root.mkdir("var", Permissions::default()).ok();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Essential directories created\n");
            uart_write_str("[VFS] Mounting ramfs as root...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            println!("[VFS] Essential directories created");
            println!("[VFS] Mounting ramfs as root...");
        }
        
        // Mount as root filesystem
        get_vfs().write().mount_root(Arc::new(ramfs)).unwrap();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Mounted ramfs as root filesystem\n");
            uart_write_str("[VFS] Creating devfs...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            println!("[VFS] Mounted ramfs as root filesystem");
            println!("[VFS] Creating devfs...");
        }
        
        // Mount special filesystems
        let devfs = devfs::DevFs::new();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Mounting devfs at /dev...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Mounting devfs at /dev...");
        
        get_vfs().write().mount("/dev".into(), Arc::new(devfs)).ok();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Mounted devfs at /dev\n");
            uart_write_str("[VFS] Creating procfs...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            println!("[VFS] Mounted devfs at /dev");
            println!("[VFS] Creating procfs...");
        }
        
        let procfs = procfs::ProcFs::new();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Mounting procfs at /proc...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Mounting procfs at /proc...");
        
        get_vfs().write().mount("/proc".into(), Arc::new(procfs)).ok();
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Mounted procfs at /proc\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Mounted procfs at /proc");
        
        // Create subdirectories
        if let Ok(usr) = root.lookup("usr") {
            usr.mkdir("bin", Permissions::default()).ok();
            usr.mkdir("sbin", Permissions::default()).ok();
            usr.mkdir("lib", Permissions::default()).ok();
            usr.mkdir("local", Permissions::default()).ok();
        }
        
        if let Ok(var) = root.lookup("var") {
            var.mkdir("log", Permissions::default()).ok();
            var.mkdir("run", Permissions::default()).ok();
            var.mkdir("tmp", Permissions::default()).ok();
        }
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[VFS] Virtual Filesystem initialized with 3 filesystems\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[VFS] Virtual Filesystem initialized with 3 filesystems");
    }
    
    #[cfg(not(feature = "alloc"))]
    {
        println!("[VFS] Skipping VFS initialization (no alloc)");
    }
}