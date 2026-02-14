//! Remote Procedure Call (RPC) Framework
//!
//! High-level RPC abstraction built on top of the IPC system.
//!
//! Provides method-based RPC with service discovery and marshaling.

#![allow(static_mut_refs)]

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use spin::RwLock;

use super::{sync_receive, sync_send, EndpointId, IpcError, Message, SmallMessage};

/// RPC method identifier
pub type MethodId = u32;

/// RPC request ID for tracking requests/responses
pub type RequestId = u64;

/// RPC message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcMessageType {
    Request = 1,
    Response = 2,
    Error = 3,
}

/// RPC error
#[derive(Debug, Clone)]
pub struct RpcError {
    pub request_id: RequestId,
    pub error_code: i32,
    pub message: String,
}

impl From<IpcError> for RpcError {
    fn from(err: IpcError) -> Self {
        let error_code = match err {
            IpcError::InvalidCapability => -1,
            IpcError::PermissionDenied => -2,
            IpcError::WouldBlock => -3,
            IpcError::Timeout => -4,
            IpcError::ProcessNotFound => -5,
            IpcError::EndpointNotFound => -6,
            IpcError::MessageTooLarge => -7,
            IpcError::OutOfMemory => -8,
            IpcError::RateLimitExceeded => -9,
            IpcError::InvalidMessage => -10,
            IpcError::ChannelFull => -11,
            IpcError::ChannelEmpty => -12,
            IpcError::EndpointBusy => -13,
            IpcError::InvalidMemoryRegion => -14,
            IpcError::ResourceBusy => -15,
            IpcError::NotInitialized => -16,
        };
        RpcError {
            request_id: 0,
            error_code,
            message: alloc::format!("{:?}", err),
        }
    }
}

/// RPC service handler trait
pub trait RpcService: Send + Sync {
    /// Get service name
    fn name(&self) -> &str;

    /// Handle RPC method call
    fn handle_method(&self, method_id: MethodId, params: &[u8]) -> Result<Vec<u8>, RpcError>;

    /// Get list of supported methods
    fn methods(&self) -> Vec<MethodId>;
}

/// RPC client for making remote calls
pub struct RpcClient {
    endpoint_id: EndpointId,
    next_request_id: RwLock<u64>,
}

impl RpcClient {
    /// Create new RPC client
    pub fn new(endpoint_id: EndpointId) -> Self {
        Self {
            endpoint_id,
            next_request_id: RwLock::new(1),
        }
    }

    /// Make synchronous RPC call
    ///
    /// Sends an RPC request and waits for the response.
    pub fn call(&self, method_id: MethodId, params: Vec<u8>) -> Result<Vec<u8>, RpcError> {
        let request_id = {
            let mut next = self.next_request_id.write();
            let id = *next;
            *next += 1;
            id
        };

        // Build RPC request message
        // For small params (≤24 bytes), pack into SmallMessage data registers
        if params.len() <= 24 {
            let mut msg = SmallMessage::new(0, RpcMessageType::Request as u32);
            msg.data[0] = request_id;
            msg.data[1] = method_id as u64;
            msg.data[2] = params.len() as u64;

            // Pack params into remaining data space (3 bytes per u64)
            for (i, chunk) in params.chunks(8).enumerate() {
                if i + 3 < 4 {
                    // data[3] available
                    let mut value = 0u64;
                    for (j, &byte) in chunk.iter().enumerate() {
                        value |= (byte as u64) << (j * 8);
                    }
                    msg.data[3] = value;
                }
            }

            // Send request
            sync_send(Message::Small(msg), self.endpoint_id)?;

            // Wait for response
            let response = sync_receive(self.endpoint_id)?;

            match response {
                Message::Small(resp_msg) => {
                    // Extract response data
                    let resp_len = resp_msg.data[2] as usize;
                    let mut result = Vec::with_capacity(resp_len);

                    // Unpack response from data[3]
                    let value = resp_msg.data[3];
                    for i in 0..resp_len.min(8) {
                        result.push(((value >> (i * 8)) & 0xFF) as u8);
                    }

                    Ok(result)
                }
                Message::Large(_) => {
                    // For now, reject large responses in small calls
                    Err(RpcError {
                        request_id,
                        error_code: -100,
                        message: "Unexpected large response".to_string(),
                    })
                }
            }
        } else {
            // For larger params, we'd use LargeMessage
            // For now, return error
            Err(RpcError {
                request_id,
                error_code: -101,
                message: "Large RPC calls not yet implemented".to_string(),
            })
        }
    }
}

/// RPC server for handling incoming calls
pub struct RpcServer {
    endpoint_id: EndpointId,
    services: RwLock<BTreeMap<String, alloc::boxed::Box<dyn RpcService>>>,
}

impl RpcServer {
    /// Create new RPC server
    pub fn new(endpoint_id: EndpointId) -> Self {
        Self {
            endpoint_id,
            services: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a service
    pub fn register_service(&self, service: alloc::boxed::Box<dyn RpcService>) {
        let name = service.name().to_string();
        self.services.write().insert(name, service);
    }

    /// Process incoming RPC requests (call in a loop)
    ///
    /// Receives one request, dispatches to appropriate service, and sends
    /// response.
    pub fn process_requests(&self) -> Result<(), RpcError> {
        // Receive incoming request
        let request = sync_receive(self.endpoint_id).map_err(RpcError::from)?;

        match request {
            Message::Small(msg) => {
                // Extract RPC request fields
                let request_id = msg.data[0];
                let method_id = msg.data[1] as u32;
                let params_len = msg.data[2] as usize;

                // Unpack parameters
                let mut params = Vec::with_capacity(params_len);
                let value = msg.data[3];
                for i in 0..params_len.min(8) {
                    params.push(((value >> (i * 8)) & 0xFF) as u8);
                }

                // TODO(phase3): Optimize service dispatch with direct method_id lookup
                let services = self.services.read();

                // Find service that handles this method
                let mut result = Vec::new();
                let mut found = false;

                for service in services.values() {
                    if service.methods().contains(&method_id) {
                        match service.handle_method(method_id, &params) {
                            Ok(response_data) => {
                                result = response_data;
                                found = true;
                                break;
                            }
                            Err(err) => {
                                // Send error response
                                let error_msg = SmallMessage::new(0, RpcMessageType::Error as u32)
                                    .with_data(0, request_id)
                                    .with_data(1, err.error_code as u64);

                                sync_send(Message::Small(error_msg), self.endpoint_id)
                                    .map_err(RpcError::from)?;
                                return Ok(());
                            }
                        }
                    }
                }

                if !found {
                    // Method not found - send error
                    let error_msg = SmallMessage::new(0, RpcMessageType::Error as u32)
                        .with_data(0, request_id)
                        .with_data(1, -404i64 as u64);

                    sync_send(Message::Small(error_msg), self.endpoint_id)
                        .map_err(RpcError::from)?;
                    return Ok(());
                }

                // Pack result into response
                let mut response_msg = SmallMessage::new(0, RpcMessageType::Response as u32);
                response_msg.data[0] = request_id;
                response_msg.data[1] = method_id as u64;
                response_msg.data[2] = result.len() as u64;

                // Pack response data
                if result.len() <= 8 {
                    let mut value = 0u64;
                    for (i, &byte) in result.iter().enumerate() {
                        value |= (byte as u64) << (i * 8);
                    }
                    response_msg.data[3] = value;
                }

                // Send response
                sync_send(Message::Small(response_msg), self.endpoint_id)
                    .map_err(RpcError::from)?;

                Ok(())
            }
            Message::Large(_) => {
                // Large message RPC not yet implemented
                Err(RpcError {
                    request_id: 0,
                    error_code: -102,
                    message: "Large RPC requests not yet implemented".to_string(),
                })
            }
        }
    }
}

/// RPC service registry for discovery
pub struct RpcRegistry {
    services: RwLock<BTreeMap<String, EndpointId>>,
}

impl RpcRegistry {
    /// Create new registry
    pub fn new() -> Self {
        Self {
            services: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a service by name
    pub fn register(&self, name: String, endpoint: EndpointId) {
        self.services.write().insert(name, endpoint);
    }

    /// Lookup service by name
    pub fn lookup(&self, name: &str) -> Option<EndpointId> {
        self.services.read().get(name).copied()
    }

    /// List all registered services
    pub fn list_services(&self) -> Vec<String> {
        self.services.read().keys().cloned().collect()
    }
}

impl Default for RpcRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global RPC registry
static GLOBAL_REGISTRY: RwLock<Option<RpcRegistry>> = RwLock::new(None);

/// Initialize RPC framework
pub fn init() {
    *GLOBAL_REGISTRY.write() = Some(RpcRegistry::new());
    crate::println!("[RPC] RPC framework initialized (stub)");
}

/// Get global RPC registry
pub fn get_registry() -> &'static RpcRegistry {
    // SAFETY: REGISTRY_STORAGE is a function-local static mut Option lazily
    // initialized on first access. The is_none() check ensures it is written at
    // most once. The returned reference has 'static lifetime because
    // function-local statics persist for the program's duration. The
    // RpcRegistry uses internal Mutex for thread safety.
    unsafe {
        static mut REGISTRY_STORAGE: Option<RpcRegistry> = None;

        if REGISTRY_STORAGE.is_none() {
            REGISTRY_STORAGE = Some(RpcRegistry::new());
        }

        // is_none() check above guarantees Some
        REGISTRY_STORAGE
            .as_ref()
            .expect("RPC registry not initialized")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_rpc_registry() {
        let registry = RpcRegistry::new();
        registry.register(String::from("test_service"), EndpointId(42));

        let found = registry.lookup("test_service");
        assert_eq!(found, Some(EndpointId(42)));
    }
}
