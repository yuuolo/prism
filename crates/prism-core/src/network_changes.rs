//! Windows notifications are hints: callbacks only wake the async scanner.
//! Route/interface queries and repairs never run on a Windows callback thread.
use std::{ffi::c_void, sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}}};
use anyhow::{bail, Result};
use tokio::sync::Notify;
use windows_sys::Win32::{Foundation::HANDLE, NetworkManagement::IpHelper::*, Networking::WinSock::AF_INET};

struct Signals {
    tun: u32,
    current: AtomicU32,
    invalidated: AtomicBool,
    changed: Notify,
}
impl Signals {
    fn signal(&self, index: u32, removed: bool) {
        if index == 0 || index == self.tun { return; }
        if removed && index == self.current.load(Ordering::Acquire) {
            // Preserve a quick delete/add even if the next scan sees the same
            // address and gateway. Notify's coalescing must not lose this fact.
            self.invalidated.store(true, Ordering::Release);
        }
        self.changed.notify_one();
    }
}

// A Box keeps the callback context address stable when Registration moves.
// A usize stores the opaque OS handle without claiming ownership via CloseHandle.
struct Registration { handle: usize, context: Option<Box<Arc<Signals>>> }
impl Drop for Registration {
    fn drop(&mut self) {
        // Called outside callbacks, without any lock/callback dependency.
        // Windows waits for outstanding callbacks before successful cancellation.
        let error = unsafe { CancelMibChangeNotify2(self.handle as HANDLE) };
        if error != 0 {
            tracing::warn!("network notification cancellation failed: {error}");
            // If Windows cannot confirm cancellation, keep the context alive.
            // Leaking this small allocation is safer than a late callback UAF.
            if let Some(context) = self.context.take() { std::mem::forget(context); }
        }
    }
}

unsafe extern "system" fn route_changed(context: *const c_void, row: *const MIB_IPFORWARD_ROW2, kind: MIB_NOTIFICATION_TYPE) {
    if context.is_null() || row.is_null() { return; }
    // Only the identity/prefix fields supplied by this API are consumed. Never
    // retain the borrowed row, nor infer full interface state from a callback.
    let row = unsafe { &*row };
    if row.DestinationPrefix.PrefixLength != 0 { return; }
    let signals = unsafe { &*(context as *const Arc<Signals>) };
    signals.signal(row.InterfaceIndex, kind == MibDeleteInstance);
}
unsafe extern "system" fn interface_changed(context: *const c_void, row: *const MIB_IPINTERFACE_ROW, kind: MIB_NOTIFICATION_TYPE) {
    if context.is_null() || row.is_null() { return; }
    let row = unsafe { &*row };
    let signals = unsafe { &*(context as *const Arc<Signals>) };
    signals.signal(row.InterfaceIndex, kind == MibDeleteInstance);
}

pub struct Changes { signals: Arc<Signals>, _registrations: Vec<Registration> }
impl Changes {
    pub fn register(tun: u32, current: u32) -> Result<Self> {
        let signals = Arc::new(Signals { tun, current: current.into(), invalidated: false.into(), changed: Notify::new() });
        let mut registrations = Vec::new();
        for route in [true, false] {
            let context = Box::new(signals.clone());
            let ptr = (&*context as *const Arc<Signals>).cast();
            let mut handle = std::ptr::null_mut();
            let error = unsafe {
                if route { NotifyRouteChange2(AF_INET, Some(route_changed), ptr, 0, &mut handle) }
                else { NotifyIpInterfaceChange(AF_INET, Some(interface_changed), ptr, 0, &mut handle) }
            };
            if error != 0 { bail!("network notification registration failed: {error}"); }
            registrations.push(Registration { handle: handle as usize, context: Some(context) });
        }
        Ok(Self { signals, _registrations: registrations })
    }
    pub async fn changed(&self) { self.signals.changed.notified().await; }
    pub fn take_invalidation(&self) -> bool { self.signals.invalidated.swap(false, Ordering::AcqRel) }
    pub fn applied(&self, current: u32) { self.signals.current.store(current, Ordering::Release); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rapid_reconnect_is_latched_and_tun_routes_are_ignored() {
        let signals = Arc::new(Signals { tun: 99, current: 7.into(), invalidated: false.into(), changed: Notify::new() });
        let changes = Changes { signals: signals.clone(), _registrations: vec![] };
        let context = (&signals as *const Arc<Signals>).cast();
        let mut route: MIB_IPFORWARD_ROW2 = unsafe { std::mem::zeroed() };
        route.InterfaceIndex = 99;
        unsafe { route_changed(context, &route, MibDeleteInstance); }
        route.InterfaceIndex = 7; route.DestinationPrefix.PrefixLength = 1;
        unsafe { route_changed(context, &route, MibDeleteInstance); }
        assert!(!changes.take_invalidation());
        assert!(tokio::time::timeout(std::time::Duration::from_millis(10), changes.changed()).await.is_err());
        route.DestinationPrefix.PrefixLength = 0;
        unsafe { route_changed(context, &route, MibDeleteInstance); route_changed(context, &route, MibAddInstance); }
        changes.changed().await;
        assert!(changes.take_invalidation()); assert!(!changes.take_invalidation());
        changes.applied(8);
        unsafe { route_changed(context, &route, MibDeleteInstance); }
        assert!(!changes.take_invalidation(), "deleting the old interface must not invalidate the new one");
        let mut interface: MIB_IPINTERFACE_ROW = unsafe { std::mem::zeroed() };
        interface.InterfaceIndex = 8;
        unsafe { interface_changed(context, &interface, MibDeleteInstance); }
        assert!(changes.take_invalidation());
        unsafe { interface_changed(context, std::ptr::null(), MibInitialNotification); }
        assert!(!changes.take_invalidation());
    }
    #[test]
    fn native_registration_and_cancellation_are_read_only_and_repeatable() {
        // No adapter/routing changes, and no administrator privileges required.
        for _ in 0..8 { drop(Changes::register(u32::MAX, 0).unwrap()); }
    }
}
