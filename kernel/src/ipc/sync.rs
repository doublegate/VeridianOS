//! Synchronous IPC implementation
//!
//! Provides blocking send/receive operations with direct handoff between
//! processes.

// Synchronous IPC -- exercised via syscall IPC paths
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    message::Message,
};
use crate::arch::entropy::read_timestamp;
#[cfg(test)]
use crate::process::ProcessId;

/// Statistics for synchronous IPC
pub struct SyncIpcStats {
    pub send_count: AtomicU64,
    pub receive_count: AtomicU64,
    pub fast_path_count: AtomicU64,
    pub slow_path_count: AtomicU64,
    pub avg_latency_cycles: AtomicU64,
}

static SYNC_STATS: SyncIpcStats = SyncIpcStats {
    send_count: AtomicU64::new(0),
    receive_count: AtomicU64::new(0),
    fast_path_count: AtomicU64::new(0),
    slow_path_count: AtomicU64::new(0),
    avg_latency_cycles: AtomicU64::new(0),
};

/// Simple send message function for tests
#[cfg(test)]
pub fn send_message(msg: Message, target_endpoint: u64) -> Result<()> {
    sync_send(msg, target_endpoint)
}

/// Synchronous message send: the message is queued on the endpoint,
/// waiting for room while its queue is full (a signal ends the wait).
pub fn sync_send(msg: Message, target_endpoint: u64) -> Result<()> {
    let start = read_timestamp();
    SYNC_STATS.send_count.fetch_add(1, Ordering::Relaxed);
    SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
    validate_send_capability(&msg, target_endpoint)?;
    send_to(target_endpoint, msg)?;
    update_latency_stats(start);
    Ok(())
}

/// Synchronous message receive: sleeps until a message is available, the
/// endpoint closes or a signal must be acted on (N-84: it yielded once and
/// returned an empty message). The caller checks the RECEIVE right.
pub fn sync_receive(endpoint: u64) -> Result<Message> {
    let start = read_timestamp();
    SYNC_STATS.receive_count.fetch_add(1, Ordering::Relaxed);
    let msg = receive_from(endpoint)?;
    update_latency_stats(start);
    Ok(msg)
}

/// Queue `msg` on endpoint `id` of the IPC registry, where endpoints are
/// created. The send path used to look them up in a second registry that
/// nothing filled, so it reached no endpoint (N-47).
#[cfg(feature = "alloc")]
fn send_to(id: u64, msg: Message) -> Result<()> {
    let endpoint = super::registry::lookup_endpoint(id)?;
    let sender = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(crate::process::ProcessId(0));
    endpoint.send_sync(msg, sender)
}

#[cfg(not(feature = "alloc"))]
fn send_to(_id: u64, _msg: Message) -> Result<()> {
    Err(IpcError::OutOfMemory)
}

/// The next message of endpoint `id`, sleeping until there is one.
#[cfg(feature = "alloc")]
fn receive_from(id: u64) -> Result<Message> {
    let endpoint = super::registry::lookup_endpoint(id)?;
    let receiver = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(crate::process::ProcessId(0));
    endpoint.receive_sync(receiver)
}

#[cfg(not(feature = "alloc"))]
fn receive_from(_id: u64) -> Result<Message> {
    Err(IpcError::OutOfMemory)
}

/// An outstanding call (N-80): the endpoint called, and the reply once a
/// server has given it.
#[cfg(feature = "alloc")]
struct PendingCall {
    endpoint: u64,
    reply: Option<Message>,
}

/// Outstanding calls by caller thread id. A reply goes into the call's own
/// slot here, never through an endpoint: endpoint numbers and thread or
/// process ids are different namespaces, and no ordinary send can reach a
/// slot. A thread has at most one call outstanding.
#[cfg(feature = "alloc")]
static PENDING_CALLS: spin::Mutex<alloc::collections::BTreeMap<u64, PendingCall>> =
    spin::Mutex::new(alloc::collections::BTreeMap::new());

/// Woken whenever a reply is stored.
#[cfg(feature = "alloc")]
static REPLIES: crate::sched::dispatch::WaitQueue = crate::sched::dispatch::WaitQueue::new();

/// Call operation (send and wait for reply). The caller is identified to
/// the server by its thread id (how a server learns it is part of the
/// reply-capability design, v0.28).
pub fn sync_call(request: Message, target: u64) -> Result<Message> {
    #[cfg(feature = "alloc")]
    {
        let me = crate::process::current_thread()
            .ok_or(IpcError::ProcessNotFound)?
            .tid
            .0;
        {
            let mut pending = PENDING_CALLS.lock();
            if pending.contains_key(&me) {
                return Err(IpcError::EndpointBusy);
            }
            pending.insert(
                me,
                PendingCall {
                    endpoint: target,
                    reply: None,
                },
            );
        }
        let result = sync_send(request, target).and_then(|()| {
            let has_reply = || {
                PENDING_CALLS
                    .lock()
                    .get(&me)
                    .is_some_and(|c| c.reply.is_some())
            };
            crate::sched::dispatch::wait_event(&REPLIES, None, has_reply)
                .map_err(|_| IpcError::Interrupted)
        });
        // Whatever the outcome the call is over: a later reply finds no
        // entry and is refused.
        let call = PENDING_CALLS.lock().remove(&me);
        result?;
        call.and_then(|c| c.reply).ok_or(IpcError::InvalidMessage)
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (request, target);
        Err(IpcError::OutOfMemory)
    }
}

/// Store `reply` for `caller`'s outstanding call if the holder of
/// `cap_space` may answer it: `cap` names the endpoint called with RECEIVE
/// rights, and the call has no reply yet (one reply per call).
#[cfg(feature = "alloc")]
fn authorize_reply(
    pending: &mut alloc::collections::BTreeMap<u64, PendingCall>,
    caller: u64,
    cap: crate::cap::CapabilityToken,
    cap_space: &crate::cap::space::CapabilitySpace,
    reply: Message,
) -> Result<()> {
    let call = pending.get_mut(&caller).ok_or(IpcError::InvalidMessage)?;
    if call.reply.is_some() {
        return Err(IpcError::InvalidMessage);
    }
    crate::cap::ipc_integration::check_endpoint_permission(
        cap,
        cap_space,
        call.endpoint,
        crate::cap::ipc_integration::IpcRights::RECEIVE,
    )?;
    call.reply = Some(reply);
    Ok(())
}

/// Reply to the outstanding call of thread `caller`. Only a server of the
/// endpoint called may reply (N-80): `reply`'s capability must name that
/// endpoint with RECEIVE rights. Any process could reply to any blocked
/// one before, and replies went to an endpoint numbered by the caller's
/// pid, a number any endpoint could have.
pub fn sync_reply(reply: Message, caller: u64) -> Result<()> {
    #[cfg(feature = "alloc")]
    {
        let process = crate::process::current_process().ok_or(IpcError::ProcessNotFound)?;
        let space = process.capability_space.lock();
        let cap = crate::cap::CapabilityToken::from_u64(reply.capability());
        authorize_reply(&mut PENDING_CALLS.lock(), caller, cap, &space, reply)?;
        drop(space);
        REPLIES.wake_all();
        Ok(())
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (reply, caller);
        Err(IpcError::OutOfMemory)
    }
}

/// Validate send capability
fn validate_send_capability(msg: &Message, endpoint_id: u64) -> Result<()> {
    let cap_id = msg.capability();

    // Get current process's capability space
    let current_process = crate::process::current_process().ok_or(IpcError::ProcessNotFound)?;
    let cap_space = current_process.capability_space.lock();

    // Convert capability ID to token
    let cap_token = crate::cap::CapabilityToken::from_u64(cap_id);

    // The capability must name this endpoint and carry SEND (N-80): one
    // for another object, or another endpoint, does not let the holder send
    // here.
    crate::cap::ipc_integration::check_endpoint_permission(
        cap_token,
        &cap_space,
        endpoint_id,
        crate::cap::ipc_integration::IpcRights::SEND,
    )?;

    Ok(())
}

/// Update latency statistics
fn update_latency_stats(start_cycles: u64) {
    let elapsed = read_timestamp() - start_cycles;
    let count = SYNC_STATS.send_count.load(Ordering::Relaxed)
        + SYNC_STATS.receive_count.load(Ordering::Relaxed);
    let current_avg = SYNC_STATS.avg_latency_cycles.load(Ordering::Relaxed);

    // Calculate new average
    let new_avg = if count > 1 {
        (current_avg * (count - 1) + elapsed) / count
    } else {
        elapsed
    };

    SYNC_STATS
        .avg_latency_cycles
        .store(new_avg, Ordering::Relaxed);

    // Also record in global performance stats
    let is_fast_path = SYNC_STATS.fast_path_count.load(Ordering::Relaxed)
        > SYNC_STATS.slow_path_count.load(Ordering::Relaxed);
    crate::ipc::perf::IPC_PERF_STATS.record_operation(elapsed, is_fast_path);
}

/// Get synchronous IPC statistics
pub fn get_sync_stats() -> SyncStatsSummary {
    SyncStatsSummary {
        send_count: SYNC_STATS.send_count.load(Ordering::Relaxed),
        receive_count: SYNC_STATS.receive_count.load(Ordering::Relaxed),
        fast_path_count: SYNC_STATS.fast_path_count.load(Ordering::Relaxed),
        slow_path_count: SYNC_STATS.slow_path_count.load(Ordering::Relaxed),
        avg_latency_cycles: SYNC_STATS.avg_latency_cycles.load(Ordering::Relaxed),
        fast_path_percentage: {
            let fast = SYNC_STATS.fast_path_count.load(Ordering::Relaxed);
            let total = fast + SYNC_STATS.slow_path_count.load(Ordering::Relaxed);
            if total > 0 {
                (fast * 100) / total
            } else {
                0
            }
        },
    }
}

pub struct SyncStatsSummary {
    pub send_count: u64,
    pub receive_count: u64,
    pub fast_path_count: u64,
    pub slow_path_count: u64,
    pub avg_latency_cycles: u64,
    pub fast_path_percentage: u64,
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    /// N-80: a reply needs an outstanding call from that caller, a
    /// RECEIVE capability for the endpoint it called, and no reply yet.
    #[cfg(feature = "alloc")]
    #[test]
    fn reply_requires_the_called_endpoint_and_an_outstanding_call() {
        use alloc::{collections::BTreeMap, sync::Arc};

        use crate::cap::{ipc_integration::IpcRights, manager::cap_manager, object::ObjectRef};
        let space = crate::cap::space::CapabilitySpace::new();
        let ep = Arc::new(crate::ipc::Endpoint::new(ProcessId(3)));
        let ep_id = ep.id();
        let server = cap_manager()
            .create_capability(
                ObjectRef::Endpoint { endpoint: ep },
                IpcRights::RECEIVE,
                &space,
            )
            .unwrap();
        let other = Arc::new(crate::ipc::Endpoint::new(ProcessId(3)));
        let wrong = cap_manager()
            .create_capability(
                ObjectRef::Endpoint { endpoint: other },
                IpcRights::RECEIVE,
                &space,
            )
            .unwrap();
        let msg = || Message::Small(crate::ipc::SmallMessage::new(0, 7));
        let mut pending = BTreeMap::new();
        // No call outstanding.
        assert!(authorize_reply(&mut pending, 40, server, &space, msg()).is_err());
        pending.insert(
            40,
            PendingCall {
                endpoint: ep_id,
                reply: None,
            },
        );
        // A capability for another endpoint does not authorize the reply.
        assert!(authorize_reply(&mut pending, 40, wrong, &space, msg()).is_err());
        assert!(pending[&40].reply.is_none());
        assert!(authorize_reply(&mut pending, 40, server, &space, msg()).is_ok());
        assert!(
            pending[&40].reply.is_some(),
            "stored in the call's own slot"
        );
        // One reply per call.
        assert!(authorize_reply(&mut pending, 40, server, &space, msg()).is_err());
    }

    #[test]
    fn test_sync_stats() {
        let stats = get_sync_stats();
        assert_eq!(stats.send_count, 0);
        assert_eq!(stats.receive_count, 0);
        assert_eq!(stats.fast_path_percentage, 0);
    }

    /// N-47: an endpoint created in the registry is the one the send and
    /// receive paths reach (they used a second registry nothing filled).
    #[cfg(feature = "alloc")]
    #[test]
    fn endpoint_from_registry_round_trips_through_send_path() {
        crate::ipc::registry::init();
        let (id, _cap) = crate::ipc::registry::create_endpoint(ProcessId(1)).unwrap();
        send_to(id, Message::small(0, 7)).unwrap();
        let received = receive_from(id).unwrap();
        assert_eq!(received.opcode(), 7);
    }
}
