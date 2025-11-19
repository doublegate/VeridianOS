//! Remote Procedure Call (RPC) Framework
//!
//! High-level RPC abstraction built on top of the IPC system.
//!
//! NOTE: This is a framework stub showing the intended RPC architecture.
//! Full implementation requires integration with the IPC message passing API.

use super::EndpointId;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::collections::BTreeMap;
use spin::RwLock;

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
    /// TODO: Implement full RPC call using IPC message passing
    pub fn call(&self, method_id: MethodId, _params: Vec<u8>) -> Result<Vec<u8>, RpcError> {
        let request_id = {
            let mut next = self.next_request_id.write();
            let id = *next;
            *next += 1;
            id
        };

        // Stub - return empty response
        crate::println!("[RPC] Stub call to endpoint {:?}, method {}", self.endpoint_id, method_id);

        Ok(Vec::new())
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
    /// TODO: Implement full request processing using IPC
    pub fn process_requests(&self) -> Result<(), ()> {
        crate::println!("[RPC] Stub process_requests for endpoint {:?}", self.endpoint_id);
        Ok(())
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
    unsafe {
        static mut REGISTRY_STORAGE: Option<RpcRegistry> = None;

        if REGISTRY_STORAGE.is_none() {
            REGISTRY_STORAGE = Some(RpcRegistry::new());
        }

        REGISTRY_STORAGE.as_ref().unwrap()
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
