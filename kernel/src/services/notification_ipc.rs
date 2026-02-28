//! Notification IPC Service
//!
//! Provides an IPC endpoint for desktop notification delivery. User-space
//! applications send notification messages to the well-known notification
//! endpoint, and this service dispatches them to the desktop notification
//! manager for rendering as toast popups.

#![allow(dead_code)]

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    desktop::notification::{self, NotificationUrgency},
    error::KernelError,
    services::desktop_ipc::DESKTOP_NOTIFICATION_ENDPOINT,
};

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Type of notification IPC message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NotificationMessageType {
    /// Post a new notification (returns notification ID).
    Notify = 0,
    /// Dismiss a specific notification by ID.
    Dismiss = 1,
    /// Dismiss all active notifications.
    DismissAll = 2,
    /// Query the count of active notifications.
    GetActive = 3,
}

impl NotificationMessageType {
    /// Convert a raw u8 to a message type, if valid.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Notify),
            1 => Some(Self::Dismiss),
            2 => Some(Self::DismissAll),
            3 => Some(Self::GetActive),
            _ => None,
        }
    }
}

/// A notification IPC message sent from user-space to the notification service.
#[derive(Debug, Clone)]
pub struct NotificationMessage {
    /// The type of operation requested.
    pub msg_type: NotificationMessageType,
    /// Notification summary / title (used by `Notify`).
    pub summary: String,
    /// Notification body text (used by `Notify`).
    pub body: String,
    /// Urgency level: 0 = Low, 1 = Normal, 2 = Critical (used by `Notify`).
    pub urgency: u8,
    /// Name of the sending application (used by `Notify`).
    pub app_name: String,
    /// Notification ID (used by `Dismiss`).
    pub notification_id: u32,
}

impl NotificationMessage {
    /// Create a `Notify` message.
    pub fn new_notify(summary: &str, body: &str, urgency: u8, app_name: &str) -> Self {
        Self {
            msg_type: NotificationMessageType::Notify,
            summary: String::from(summary),
            body: String::from(body),
            urgency,
            app_name: String::from(app_name),
            notification_id: 0,
        }
    }

    /// Create a `Dismiss` message for a specific notification.
    pub fn new_dismiss(id: u32) -> Self {
        Self {
            msg_type: NotificationMessageType::Dismiss,
            summary: String::new(),
            body: String::new(),
            urgency: 1,
            app_name: String::new(),
            notification_id: id,
        }
    }

    /// Create a `DismissAll` message.
    pub fn new_dismiss_all() -> Self {
        Self {
            msg_type: NotificationMessageType::DismissAll,
            summary: String::new(),
            body: String::new(),
            urgency: 1,
            app_name: String::new(),
            notification_id: 0,
        }
    }

    /// Create a `GetActive` query message.
    pub fn new_get_active() -> Self {
        Self {
            msg_type: NotificationMessageType::GetActive,
            summary: String::new(),
            body: String::new(),
            urgency: 1,
            app_name: String::new(),
            notification_id: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// IPC Server
// ---------------------------------------------------------------------------

/// Whether the notification IPC server has been initialized.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Notification IPC server that handles incoming notification messages
/// and dispatches them to the desktop notification manager.
pub struct NotificationIpcServer {
    /// The well-known endpoint ID this server is bound to.
    endpoint_id: u64,
}

impl NotificationIpcServer {
    /// Create a new notification IPC server instance.
    pub fn new() -> Self {
        Self {
            endpoint_id: DESKTOP_NOTIFICATION_ENDPOINT,
        }
    }

    /// Initialize the notification IPC server by registering the endpoint.
    ///
    /// The actual endpoint registration is handled by `desktop_ipc::init()`;
    /// this method validates that the endpoint is available and marks the
    /// notification service as ready to accept messages.
    pub fn init(&self) -> Result<(), KernelError> {
        if INITIALIZED.load(Ordering::Acquire) {
            return Err(KernelError::InvalidState {
                expected: "uninitialized",
                actual: "initialized",
            });
        }

        crate::println!(
            "[NOTIFY-IPC] Notification IPC server bound to endpoint {}",
            self.endpoint_id
        );

        INITIALIZED.store(true, Ordering::Release);
        Ok(())
    }

    /// Handle an incoming notification message.
    ///
    /// Returns:
    /// - For `Notify`: the assigned notification ID.
    /// - For `Dismiss`/`DismissAll`: 0 on success.
    /// - For `GetActive`: the count of active notifications.
    pub fn handle_message(&self, msg: &NotificationMessage) -> Result<u32, KernelError> {
        match msg.msg_type {
            NotificationMessageType::Notify => {
                let urgency = NotificationUrgency::from_u8(msg.urgency);
                let id = notification::with_notification_manager(|mgr| {
                    mgr.notify(
                        msg.summary.clone(),
                        msg.body.clone(),
                        urgency,
                        msg.app_name.clone(),
                    )
                })
                .ok_or(KernelError::InvalidState {
                    expected: "notification_manager_initialized",
                    actual: "not_initialized",
                })?;

                Ok(id)
            }

            NotificationMessageType::Dismiss => {
                notification::dismiss(msg.notification_id);
                Ok(0)
            }

            NotificationMessageType::DismissAll => {
                notification::dismiss_all();
                Ok(0)
            }

            NotificationMessageType::GetActive => {
                let count =
                    notification::with_notification_manager(|mgr| mgr.active_count()).unwrap_or(0);
                Ok(count as u32)
            }
        }
    }
}

impl Default for NotificationIpcServer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Module-level initialization
// ---------------------------------------------------------------------------

/// Initialize the notification IPC service.
pub fn init() -> Result<(), KernelError> {
    let server = NotificationIpcServer::new();
    server.init()?;
    Ok(())
}

/// Check whether the notification IPC service has been initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}
