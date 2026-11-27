//! A request's stop flag, optionally shared with the authenticated GUI over the local pipe.
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{GetLastError, LocalFree, SetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Threading::{CreateEventW, OpenEventW, SetEvent, WaitForSingleObject};

#[derive(Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    // The requester alone has a writable handle. The GUI can only observe the event.
    event: Mutex<Option<(String, OwnedHandle)>>,
    remote: Option<OwnedHandle>,
}

const CANCELLED: &str = "Sprawl cancelled the request.";

fn event_name(key: &str) -> Vec<u16> {
    format!("Global\\sprawl-mcp-cancel-{key}").encode_utf16().chain(Some(0)).collect()
}

fn security() -> std::io::Result<EventSecurity> {
    let sid = sprawl_core::identity::account_sid()?;
    // Only this account and SYSTEM can open/signal it, independent of the token's default DACL.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)").encode_utf16().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(EventSecurity(SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 }))
}

struct EventSecurity(SECURITY_ATTRIBUTES);

impl Drop for EventSecurity {
    fn drop(&mut self) { unsafe { LocalFree(self.0.lpSecurityDescriptor); } }
}

impl Cancel {
    pub(crate) fn export(&self) -> std::io::Result<String> {
        self.export_with(crate::store::nonce)
    }

    fn export_with(&self, mut nonce: impl FnMut() -> String) -> std::io::Result<String> {
        if let Some((key, _)) = &*self.event.lock().unwrap_or_else(|e| e.into_inner()) { return Ok(key.clone()); }
        let security = security()?;
        for _ in 0..4 {
            let key = nonce();
            let name = event_name(&key);
            let h = unsafe {
                SetLastError(0);
                CreateEventW(&security.0, 1, self.flag.load(Ordering::Relaxed) as i32, name.as_ptr())
            };
            let code = unsafe { GetLastError() };
            if h.is_null() {
                // A pre-existing object can have a different type or deny our access.
                if matches!(code, ERROR_ACCESS_DENIED | ERROR_INVALID_HANDLE) { continue; }
                return Err(std::io::Error::from_raw_os_error(code as i32));
            }
            let handle = unsafe { OwnedHandle::from_raw_handle(h) };
            if code == ERROR_ALREADY_EXISTS { continue; }
            // Never make cancellation/shutdown wait on token lookup or event creation.
            let mut event = self.event.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((key, _)) = &*event { return Ok(key.clone()); }
            if self.flag.load(Ordering::Relaxed) { unsafe { SetEvent(handle.as_raw_handle()); } }
            *event = Some((key.clone(), handle));
            return Ok(key);
        }
        Err(std::io::Error::other("couldn't create a fresh cancellation signal after four attempts"))
    }

    pub(crate) fn import(key: &str) -> std::io::Result<Self> {
        if key.is_empty() || key.len() > 100 || !key.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(std::io::Error::other("invalid cancellation signal"));
        }
        let name = event_name(key);
        let h = unsafe { OpenEventW(0x0010_0000, 0, name.as_ptr()) }; // SYNCHRONIZE only
        if h.is_null() { return Err(std::io::Error::last_os_error()); }
        Ok(Self { remote: Some(unsafe { OwnedHandle::from_raw_handle(h) }), ..Self::default() })
    }

    pub fn stop(&self) {
        self.flag.store(true, Ordering::Relaxed);
        if let Some((_, event)) = &*self.event.lock().unwrap_or_else(|e| e.into_inner()) {
            unsafe { SetEvent(event.as_raw_handle()); }
        }
    }

    pub fn stopped(&self) -> bool {
        if !self.flag.load(Ordering::Relaxed) && self.remote.as_ref().is_some_and(|h| unsafe {
            WaitForSingleObject(h.as_raw_handle(), 0) == WAIT_OBJECT_0
        }) {
            self.flag.store(true, Ordering::Relaxed);
        }
        self.flag.load(Ordering::Relaxed)
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        if self.stopped() { Err(CANCELLED.into()) } else { Ok(()) }
    }

    pub(crate) fn flag(&self) -> &Arc<AtomicBool> { &self.flag }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_retries_a_collision_without_reusing_its_event() {
        let first = Cancel::default();
        let taken = first.export().unwrap();
        let next = Cancel::default();
        let mut keys = [taken.clone(), crate::store::nonce()].into_iter();
        let key = next.export_with(|| keys.next().unwrap()).unwrap();
        assert_ne!(key, taken);
        let first_peer = Cancel::import(&taken).unwrap();
        let next_peer = Cancel::import(&key).unwrap();
        next.stop();
        assert!(next_peer.stopped());
        assert!(!first_peer.stopped());
    }
}
