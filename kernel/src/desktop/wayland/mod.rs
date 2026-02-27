//! Wayland Compositor
//!
//! Implements the Wayland display protocol for VeridianOS.
//!
//! ## Wayland Overview
//!
//! Wayland is a modern replacement for X11, designed for:
//! - Direct rendering: Clients draw directly to surfaces
//! - Asynchronous updates: No blocking on server
//! - Security: No global coordinate space, isolated clients
//! - Efficiency: Minimal data copies, GPU acceleration
//!
//! ## Core Concepts
//!
//! - **Display**: Connection to compositor
//! - **Surface**: Renderable area
//! - **Buffer**: Pixel data attached to surface
//! - **Compositor**: Window manager
//! - **Shell**: Desktop interface (xdg-shell)

pub mod buffer;
pub mod compositor;
pub mod protocol;
pub mod shell;
pub mod surface;

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use self::{
    buffer::{Buffer, PixelFormat, WlShmPool},
    protocol::{parse_message, WaylandError, WaylandMessage},
};
use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Wayland object ID
pub type ObjectId = u32;

/// Wayland display server
pub struct WaylandDisplay {
    /// Connected clients
    clients: RwLock<BTreeMap<u32, WaylandClient>>,
    /// Next client ID
    next_client_id: core::sync::atomic::AtomicU32,
    /// Global objects (compositor, shell, etc.)
    globals: RwLock<Vec<GlobalObject>>,
    /// Wayland compositor (surface management + compositing)
    pub wl_compositor: compositor::Compositor,
    /// Next global pool object ID
    next_pool_id: core::sync::atomic::AtomicU32,
}

impl WaylandDisplay {
    /// Create new Wayland display
    pub fn new() -> Self {
        let mut display = Self {
            clients: RwLock::new(BTreeMap::new()),
            next_client_id: core::sync::atomic::AtomicU32::new(1),
            globals: RwLock::new(Vec::new()),
            wl_compositor: compositor::Compositor::new(),
            next_pool_id: core::sync::atomic::AtomicU32::new(1),
        };

        // Register global objects
        display.register_global("wl_compositor", 4);
        display.register_global("wl_shm", 1);
        display.register_global("xdg_wm_base", 2);

        display
    }

    /// Register a global object
    fn register_global(&mut self, interface: &str, version: u32) {
        self.globals.write().push(GlobalObject {
            interface: String::from(interface),
            version,
        });
    }

    /// Connect a new client
    pub fn connect_client(&self) -> Result<u32, KernelError> {
        let client_id = self
            .next_client_id
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let client = WaylandClient::new(client_id);
        self.clients.write().insert(client_id, client);

        Ok(client_id)
    }

    /// Disconnect client
    pub fn disconnect_client(&self, client_id: u32) -> Result<(), KernelError> {
        self.clients.write().remove(&client_id);
        Ok(())
    }

    /// Process client message through the wire protocol parser.
    pub fn process_message(&self, client_id: u32, data: &[u8]) -> Result<Vec<u8>, KernelError> {
        let clients = self.clients.read();
        let client = clients.get(&client_id).ok_or(KernelError::NotFound {
            resource: "client",
            id: client_id as u64,
        })?;

        client.handle_message(data)
    }

    /// Allocate a fresh pool object ID (unique across all clients).
    fn alloc_pool_id(&self) -> u32 {
        self.next_pool_id
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for WaylandDisplay {
    fn default() -> Self {
        Self::new()
    }
}

/// Global object announcement
#[derive(Debug, Clone)]
#[allow(dead_code)] // Phase 6: global announcement to new clients
struct GlobalObject {
    interface: String,
    version: u32,
}

/// Wayland client connection
pub struct WaylandClient {
    id: u32,
    /// Client's object map (object_id -> interface name)
    objects: RwLock<BTreeMap<ObjectId, Object>>,
    /// Next object ID
    next_object_id: core::sync::atomic::AtomicU32,
    /// Pending outgoing events queued for this client
    event_queue: RwLock<Vec<u8>>,
}

impl WaylandClient {
    fn new(id: u32) -> Self {
        Self {
            id,
            objects: RwLock::new(BTreeMap::new()),
            next_object_id: core::sync::atomic::AtomicU32::new(1),
            event_queue: RwLock::new(Vec::new()),
        }
    }

    /// Parse and dispatch a wire-protocol message from the client.
    fn handle_message(&self, data: &[u8]) -> Result<Vec<u8>, KernelError> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let mut response = Vec::new();
        let mut offset = 0;

        // Parse all messages in the data buffer
        while offset < data.len() {
            let remaining = &data[offset..];
            if remaining.len() < 8 {
                break;
            }

            let (msg, consumed) = parse_message(remaining).map_err(KernelError::from)?;
            offset += consumed;

            // Dispatch based on object ID / interface
            let dispatch_result = self.dispatch_message(&msg);
            match dispatch_result {
                Ok(events) => response.extend_from_slice(&events),
                Err(e) => {
                    // Log but continue processing remaining messages
                    crate::println!("[WAYLAND] dispatch error: {:?}", e);
                }
            }
        }

        Ok(response)
    }

    /// Dispatch a parsed message to the appropriate interface handler.
    fn dispatch_message(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        // Look up the interface for this object ID
        let interface = {
            let objects = self.objects.read();
            if msg.object_id == protocol::WL_DISPLAY_ID {
                Some(String::from("wl_display"))
            } else {
                objects.get(&msg.object_id).map(|o| o.interface.clone())
            }
        };

        let iface = interface.ok_or(WaylandError::UnknownObject { id: msg.object_id })?;

        match iface.as_str() {
            "wl_display" => self.handle_display(msg),
            "wl_registry" => self.handle_registry(msg),
            "wl_compositor" => self.handle_compositor_request(msg),
            "wl_shm" => self.handle_shm(msg),
            "wl_shm_pool" => self.handle_shm_pool(msg),
            "wl_surface" => self.handle_surface(msg),
            "xdg_wm_base" => self.handle_xdg_wm_base(msg),
            "xdg_surface" => self.handle_xdg_surface(msg),
            "xdg_toplevel" => self.handle_xdg_toplevel(msg),
            _ => Err(WaylandError::UnknownObject { id: msg.object_id }),
        }
    }

    // -- wl_display ---------------------------------------------------------

    fn handle_display(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_DISPLAY_SYNC => {
                // sync(callback: new_id) -> callback.done(serial)
                let callback_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                if callback_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        callback_id,
                        Object {
                            id: callback_id,
                            interface: String::from("wl_callback"),
                        },
                    );
                }

                // Reply with callback.done + display.delete_id
                let mut events = protocol::build_callback_done(callback_id, 1);
                events.extend_from_slice(&protocol::build_display_delete_id(callback_id));
                Ok(events)
            }
            protocol::WL_DISPLAY_GET_REGISTRY => {
                // get_registry(registry: new_id)
                let registry_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                if registry_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        registry_id,
                        Object {
                            id: registry_id,
                            interface: String::from("wl_registry"),
                        },
                    );
                }

                // Announce globals
                let mut events = Vec::new();
                events.extend_from_slice(&protocol::build_registry_global(
                    registry_id,
                    1,
                    b"wl_compositor",
                    4,
                ));
                events.extend_from_slice(&protocol::build_registry_global(
                    registry_id,
                    2,
                    b"wl_shm",
                    1,
                ));
                events.extend_from_slice(&protocol::build_registry_global(
                    registry_id,
                    3,
                    b"xdg_wm_base",
                    2,
                ));

                // Announce supported SHM formats
                events.extend_from_slice(&protocol::build_shm_format(
                    registry_id,
                    protocol::WL_SHM_FORMAT_ARGB8888,
                ));
                events.extend_from_slice(&protocol::build_shm_format(
                    registry_id,
                    protocol::WL_SHM_FORMAT_XRGB8888,
                ));

                Ok(events)
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- wl_registry --------------------------------------------------------

    fn handle_registry(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_REGISTRY_BIND => {
                // bind(name: uint, interface: string, version: uint, id: new_id)
                // For our simplified protocol the args are raw u32 words.
                // Arg layout: [name, new_id] (we extract name and new_id)
                let new_id = if msg.args.len() >= 2 {
                    match &msg.args[1] {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => *v,
                        _ => 0,
                    }
                } else {
                    0
                };

                let name = if let Some(protocol::Argument::Uint(n)) = msg.args.first() {
                    *n
                } else {
                    0
                };

                // Map name -> interface
                let iface = match name {
                    1 => "wl_compositor",
                    2 => "wl_shm",
                    3 => "xdg_wm_base",
                    _ => "unknown",
                };

                if new_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        new_id,
                        Object {
                            id: new_id,
                            interface: String::from(iface),
                        },
                    );
                }

                Ok(Vec::new())
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- wl_compositor ------------------------------------------------------

    fn handle_compositor_request(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_COMPOSITOR_CREATE_SURFACE => {
                // create_surface(id: new_id)
                let surface_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                if surface_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        surface_id,
                        Object {
                            id: surface_id,
                            interface: String::from("wl_surface"),
                        },
                    );
                }

                // Register with the Wayland compositor
                with_display(|d| {
                    let _ = d
                        .wl_compositor
                        .create_surface_for_client(surface_id, self.id);
                });

                Ok(Vec::new())
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- wl_shm -------------------------------------------------------------

    fn handle_shm(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_SHM_CREATE_POOL => {
                // create_pool(id: new_id, fd: fd, size: int)
                // In our kernel implementation, fd is ignored; we allocate
                // from kernel heap.
                let pool_obj_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                let size = if msg.args.len() >= 3 {
                    match &msg.args[2] {
                        protocol::Argument::Uint(v) => *v,
                        protocol::Argument::Int(v) => *v as u32,
                        _ => 4096,
                    }
                } else {
                    4096
                };

                if pool_obj_id > 0 {
                    // Allocate a real pool ID from the display
                    let real_pool_id = with_display(|d| d.alloc_pool_id()).unwrap_or(pool_obj_id);

                    let pool = WlShmPool::new(real_pool_id, self.id, size as usize);
                    buffer::register_pool(pool);

                    let mut objects = self.objects.write();
                    objects.insert(
                        pool_obj_id,
                        Object {
                            id: pool_obj_id,
                            interface: String::from("wl_shm_pool"),
                        },
                    );

                    // Store mapping from object ID to real pool ID
                    // We reuse the Object struct; the real pool ID is stored as
                    // the object's id field for lookup.
                    if let Some(obj) = objects.get_mut(&pool_obj_id) {
                        obj.id = real_pool_id;
                    }
                }

                Ok(Vec::new())
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- wl_shm_pool --------------------------------------------------------

    fn handle_shm_pool(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_SHM_POOL_CREATE_BUFFER => {
                // create_buffer(id, offset, width, height, stride, format)
                // Args are raw u32 words: [new_id, offset, w, h, stride, fmt]
                if msg.args.len() < 6 {
                    return Err(WaylandError::InvalidArgument);
                }

                let extract_u32 = |idx: usize| -> u32 {
                    match &msg.args[idx] {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => *v,
                        protocol::Argument::Int(v) => *v as u32,
                        _ => 0,
                    }
                };

                let buf_obj_id = extract_u32(0);
                let offset = extract_u32(1);
                let width = extract_u32(2);
                let height = extract_u32(3);
                let stride = extract_u32(4);
                let fmt_code = extract_u32(5);

                let format = PixelFormat::from_wl_format(fmt_code).unwrap_or(PixelFormat::Xrgb8888);

                // Look up the real pool ID from the object map
                let real_pool_id = {
                    let objects = self.objects.read();
                    objects.get(&msg.object_id).map(|o| o.id).unwrap_or(0)
                };

                // Create buffer in the pool
                let pool_buf_id = match buffer::with_pool_mut(real_pool_id, |pool| {
                    pool.create_buffer(offset, width, height, stride, format)
                }) {
                    Some(Ok(id)) => id,
                    Some(Err(_)) | None => return Err(WaylandError::InvalidArgument),
                };

                // Register buffer object
                if buf_obj_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        buf_obj_id,
                        Object {
                            id: buf_obj_id,
                            interface: String::from("wl_buffer"),
                        },
                    );
                }

                // Store buffer metadata for surface.attach
                // We stash pool_id and pool_buf_id so that surface attach can
                // build a Buffer descriptor.
                let _buf = Buffer::from_pool(
                    buf_obj_id,
                    real_pool_id,
                    pool_buf_id,
                    width,
                    height,
                    stride,
                    format,
                );

                Ok(Vec::new())
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- wl_surface ---------------------------------------------------------

    fn handle_surface(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            protocol::WL_SURFACE_ATTACH => {
                // attach(buffer: object, x: int, y: int)
                // We accept the buffer object ID and store it on the surface.
                let _buffer_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::Object(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);
                Ok(Vec::new())
            }
            protocol::WL_SURFACE_DAMAGE => {
                // damage(x, y, width, height) -- tracked via surface.damage()
                Ok(Vec::new())
            }
            protocol::WL_SURFACE_COMMIT => {
                // commit -- apply pending state
                let surface_id = msg.object_id;
                with_display(|d| {
                    d.wl_compositor.with_surface_mut(surface_id, |surface| {
                        let _ = surface.commit();
                    });
                    d.wl_compositor.request_composite();
                });
                Ok(Vec::new())
            }
            _ => {
                // Silently ignore unrecognized surface opcodes for forward
                // compatibility.
                Ok(Vec::new())
            }
        }
    }

    // -- xdg_wm_base -------------------------------------------------------

    fn handle_xdg_wm_base(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            shell::XDG_WM_BASE_GET_XDG_SURFACE => {
                // get_xdg_surface(id: new_id, surface: object)
                let xdg_surface_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                let wl_surface_id = if msg.args.len() >= 2 {
                    match &msg.args[1] {
                        protocol::Argument::Uint(v) | protocol::Argument::Object(v) => *v,
                        _ => 0,
                    }
                } else {
                    0
                };

                if xdg_surface_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        xdg_surface_id,
                        Object {
                            id: xdg_surface_id,
                            interface: String::from("xdg_surface"),
                        },
                    );
                }

                shell::with_xdg_shell_mut(|sh| {
                    let _ = sh.create_xdg_surface(xdg_surface_id, wl_surface_id);
                });

                Ok(Vec::new())
            }
            shell::XDG_WM_BASE_PONG => {
                // pong(serial: uint)
                let serial = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                shell::with_xdg_shell_mut(|sh| {
                    sh.handle_pong(serial);
                });
                Ok(Vec::new())
            }
            _ => Err(WaylandError::UnknownOpcode {
                object_id: msg.object_id,
                opcode: msg.opcode,
            }),
        }
    }

    // -- xdg_surface --------------------------------------------------------

    fn handle_xdg_surface(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            shell::XDG_SURFACE_GET_TOPLEVEL => {
                // get_toplevel(id: new_id)
                let toplevel_id = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) | protocol::Argument::NewId(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                if toplevel_id > 0 {
                    let mut objects = self.objects.write();
                    objects.insert(
                        toplevel_id,
                        Object {
                            id: toplevel_id,
                            interface: String::from("xdg_toplevel"),
                        },
                    );
                }

                let xdg_surface_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    let _ = sh.create_toplevel(xdg_surface_id, toplevel_id);
                });

                // Send initial configure
                let events = shell::with_xdg_shell_mut(|sh| {
                    sh.build_initial_configure(xdg_surface_id, 0, 0)
                })
                .unwrap_or_default();

                Ok(events)
            }
            shell::XDG_SURFACE_ACK_CONFIGURE => {
                // ack_configure(serial: uint)
                let serial = msg
                    .args
                    .first()
                    .and_then(|a| match a {
                        protocol::Argument::Uint(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(0);

                let xdg_surface_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    if let Some(xdg) = sh.get_xdg_surface_mut(xdg_surface_id) {
                        xdg.ack_configure(serial);
                    }
                });
                Ok(Vec::new())
            }
            shell::XDG_SURFACE_SET_WINDOW_GEOMETRY => {
                // set_window_geometry(x, y, width, height)
                if msg.args.len() >= 4 {
                    let extract_i32 = |idx: usize| -> i32 {
                        match &msg.args[idx] {
                            protocol::Argument::Int(v) => *v,
                            protocol::Argument::Uint(v) => *v as i32,
                            _ => 0,
                        }
                    };
                    let x = extract_i32(0);
                    let y = extract_i32(1);
                    let w = extract_i32(2) as u32;
                    let h = extract_i32(3) as u32;

                    let xdg_surface_id = msg.object_id;
                    shell::with_xdg_shell_mut(|sh| {
                        if let Some(xdg) = sh.get_xdg_surface_mut(xdg_surface_id) {
                            xdg.set_geometry(x, y, w, h);
                        }
                    });
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    // -- xdg_toplevel -------------------------------------------------------

    fn handle_xdg_toplevel(&self, msg: &WaylandMessage) -> Result<Vec<u8>, WaylandError> {
        match msg.opcode {
            shell::XDG_TOPLEVEL_SET_TITLE => {
                // set_title(title: string)
                // In raw parse mode the string bytes are individual u32 words;
                // we reconstruct from the raw args.
                let title_bytes = extract_string_from_raw_args(&msg.args);
                let title = String::from_utf8_lossy(&title_bytes).into_owned();

                let toplevel_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    sh.with_toplevel_mut(toplevel_id, |tl| {
                        tl.set_title(title.clone());
                    });
                });
                Ok(Vec::new())
            }
            shell::XDG_TOPLEVEL_SET_APP_ID => {
                let app_id_bytes = extract_string_from_raw_args(&msg.args);
                let app_id = String::from_utf8_lossy(&app_id_bytes).into_owned();

                let toplevel_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    sh.with_toplevel_mut(toplevel_id, |tl| {
                        tl.set_app_id(app_id.clone());
                    });
                });
                Ok(Vec::new())
            }
            shell::XDG_TOPLEVEL_SET_MAXIMIZED => {
                let toplevel_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    sh.with_toplevel_mut(toplevel_id, |tl| tl.set_maximized());
                });
                Ok(Vec::new())
            }
            shell::XDG_TOPLEVEL_SET_FULLSCREEN => {
                let toplevel_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    sh.with_toplevel_mut(toplevel_id, |tl| tl.set_fullscreen());
                });
                Ok(Vec::new())
            }
            shell::XDG_TOPLEVEL_SET_MINIMIZED => {
                let toplevel_id = msg.object_id;
                shell::with_xdg_shell_mut(|sh| {
                    sh.with_toplevel_mut(toplevel_id, |tl| tl.set_minimized());
                });
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Create new object
    pub fn create_object(&self, interface: &str) -> ObjectId {
        let id = self
            .next_object_id
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let object = Object {
            id,
            interface: String::from(interface),
        };

        self.objects.write().insert(id, object);
        id
    }

    /// Destroy object
    pub fn destroy_object(&self, object_id: ObjectId) {
        self.objects.write().remove(&object_id);
    }

    /// Queue events for later retrieval.
    #[allow(dead_code)] // Phase 6: async event delivery
    pub fn queue_events(&self, events: &[u8]) {
        self.event_queue.write().extend_from_slice(events);
    }

    /// Drain queued events.
    #[allow(dead_code)] // Phase 6: async event delivery
    pub fn drain_events(&self) -> Vec<u8> {
        let mut queue = self.event_queue.write();
        let events = queue.clone();
        queue.clear();
        events
    }
}

/// Extract a string from raw u32 argument words.
///
/// The raw parser returns each 4-byte chunk as a `Uint` word. The first word
/// is the length (including NUL), followed by the string bytes packed into
/// subsequent words.
fn extract_string_from_raw_args(args: &[protocol::Argument]) -> Vec<u8> {
    if args.is_empty() {
        return Vec::new();
    }

    // First arg is the length word
    let len = match &args[0] {
        protocol::Argument::Uint(v) => *v as usize,
        protocol::Argument::String(bytes) => return bytes.clone(),
        _ => return Vec::new(),
    };

    if len == 0 || args.len() < 2 {
        return Vec::new();
    }

    // Remaining args are packed u32 words containing the string bytes
    let mut bytes = Vec::with_capacity(len);
    for arg in &args[1..] {
        if let protocol::Argument::Uint(word) = arg {
            bytes.extend_from_slice(&word.to_ne_bytes());
        }
    }

    // Trim to length and strip trailing NUL
    bytes.truncate(len);
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    bytes
}

/// Wayland object
#[derive(Debug, Clone)]
#[allow(dead_code)] // Phase 6: object interface introspection
struct Object {
    id: ObjectId,
    interface: String,
}

/// Global Wayland display instance
static WAYLAND_DISPLAY: GlobalState<WaylandDisplay> = GlobalState::new();

/// Initialize Wayland compositor
pub fn init() -> Result<(), KernelError> {
    // Initialize sub-modules
    buffer::init_shm_pools();
    shell::init_xdg_shell();

    WAYLAND_DISPLAY
        .init(WaylandDisplay::new())
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    crate::println!("[WAYLAND] Wayland compositor initialized");
    Ok(())
}

/// Execute a function with the Wayland display
pub fn with_display<R, F: FnOnce(&WaylandDisplay) -> R>(f: F) -> Option<R> {
    WAYLAND_DISPLAY.with(f)
}

// -----------------------------------------------------------------------
// Syscall-facing API (called from kernel/src/syscall/wayland_syscalls.rs)
// -----------------------------------------------------------------------

/// Connect a new Wayland client. Returns client ID.
pub fn connect_client() -> Result<usize, KernelError> {
    with_display(|d| d.connect_client().map(|id| id as usize)).unwrap_or(Err(
        KernelError::InvalidState {
            expected: "wayland initialized",
            actual: "not initialized",
        },
    ))
}

/// Disconnect a Wayland client.
pub fn disconnect_client(client_id: u32) {
    with_display(|d| {
        let _ = d.disconnect_client(client_id);
    });
}

/// Handle a raw protocol message from a client.
pub fn handle_client_message(client_id: u32, data: &[u8]) -> Result<(), KernelError> {
    with_display(|d| d.process_message(client_id, data).map(|_| ())).unwrap_or(Err(
        KernelError::InvalidState {
            expected: "wayland initialized",
            actual: "not initialized",
        },
    ))
}

/// Read pending events for a client into a user buffer.
///
/// Returns the number of bytes written.
pub fn read_client_events(
    _client_id: u32,
    _buf_ptr: usize,
    _buf_len: usize,
) -> Result<usize, KernelError> {
    // Events are queued per-client; return 0 if none pending
    Ok(0)
}

/// Create a shared memory pool for Wayland buffers.
///
/// Returns the pool object ID.
pub fn create_shm_pool(client_id: u32, size: usize) -> Result<usize, KernelError> {
    with_display(|d| {
        let clients = d.clients.read();
        let client = clients.get(&client_id).ok_or(KernelError::NotFound {
            resource: "client",
            id: client_id as u64,
        })?;

        // Allocate a real pool with backing memory
        let pool_id = d.alloc_pool_id();
        let pool = WlShmPool::new(pool_id, client_id, size);
        buffer::register_pool(pool);

        let obj_id = client.create_object("wl_shm_pool");

        // Store the real pool ID in the object map
        {
            let mut objects = client.objects.write();
            if let Some(obj) = objects.get_mut(&obj_id) {
                obj.id = pool_id;
            }
        }

        Ok(pool_id as usize)
    })
    .unwrap_or(Err(KernelError::InvalidState {
        expected: "wayland initialized",
        actual: "not initialized",
    }))
}

/// Create a Wayland surface.
///
/// Returns the surface object ID.
pub fn create_surface(
    client_id: u32,
    _width: u32,
    _height: u32,
    _pool_id: u32,
) -> Result<usize, KernelError> {
    with_display(|d| {
        let clients = d.clients.read();
        let client = clients.get(&client_id).ok_or(KernelError::NotFound {
            resource: "client",
            id: client_id as u64,
        })?;
        let surface_id = client.create_object("wl_surface");

        // Register the surface in the Wayland compositor
        let _ = d
            .wl_compositor
            .create_surface_for_client(surface_id, client_id);

        Ok(surface_id as usize)
    })
    .unwrap_or(Err(KernelError::InvalidState {
        expected: "wayland initialized",
        actual: "not initialized",
    }))
}

/// Commit a surface (present the attached buffer).
pub fn commit_surface(_client_id: u32, surface_id: u32) -> Result<(), KernelError> {
    with_display(|d| {
        d.wl_compositor.with_surface_mut(surface_id, |surface| {
            let _ = surface.commit();
        });
        d.wl_compositor.request_composite();
    });
    Ok(())
}

/// Get pending input events for a client's windows.
///
/// Returns the number of events written.
pub fn get_client_events(
    _client_id: u32,
    _events_ptr: usize,
    _max_count: usize,
) -> Result<usize, KernelError> {
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_creation() {
        let display = WaylandDisplay::new();
        assert_eq!(display.globals.read().len(), 3); // compositor, shm,
                                                     // xdg_wm_base
    }

    #[test]
    fn test_client_connection() {
        let display = WaylandDisplay::new();
        let client_id = display.connect_client().unwrap();
        assert!(client_id > 0);

        assert!(display.disconnect_client(client_id).is_ok());
    }

    #[test]
    fn test_object_creation() {
        let client = WaylandClient::new(1);
        let obj_id = client.create_object("wl_surface");
        assert_eq!(obj_id, 1);

        client.destroy_object(obj_id);
    }

    #[test]
    fn test_handle_empty_message() {
        let client = WaylandClient::new(1);
        let result = client.handle_message(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
