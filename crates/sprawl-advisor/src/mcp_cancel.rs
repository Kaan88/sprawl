//! A request's stop flag, optionally shared with the authenticated GUI over the local pipe.
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, WAIT_OBJECT_0};
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

impl Cancel {
    pub(crate) fn export(&self) -> std::io::Result<String> {
        let mut event = self.event.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((key, _)) = &*event { return Ok(key.clone()); }
        let key = crate::store::nonce();
        let name = event_name(&key);
        let h = unsafe { CreateEventW(std::ptr::null(), 1, self.flag.load(Ordering::Relaxed) as i32, name.as_ptr()) };
        if h.is_null() { return Err(std::io::Error::last_os_error()); }
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle = unsafe { OwnedHandle::from_raw_handle(h) };
        if existed { return Err(std::io::Error::other("couldn't create a fresh cancellation signal")); }
        *event = Some((key.clone(), handle));
        Ok(key)
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
