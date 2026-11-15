//! Thread-safe access to hidapi: one library instance per process, created on first use.

use std::ffi::{CStr, CString};
use std::sync::Mutex;

use hidapi::{HidApi, HidDevice};

use crate::error::{Error, Result};

pub struct Interface {
    pub path: CString,
    pub usage_page: u16,
    pub usage: u16,
    pub product: Option<String>,
}

/// Create the hidapi instance on a thread that lives for the rest of the process.
///
/// On macOS, hidapi schedules its IOHIDManager on the run loop of the thread that calls
/// `hid_init`. Every HTTP request runs on its own short-lived thread, so creating the API there
/// left the manager pointing at a freed run loop once that thread exited; the next enumeration
/// then crashed inside `IOHIDManagerSetDeviceMatchingMultiple` (pointer-authentication trap in
/// `CFRunLoopAddSource`). A parked, never-exiting thread keeps that run loop valid.
fn create_api() -> Result<HidApi> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hid-runloop".into())
        .spawn(move || {
            let created = HidApi::new();
            let ok = created.is_ok();
            let _ = tx.send(created);
            if ok {
                loop {
                    std::thread::park();
                }
            }
        })
        .map_err(|e| Error::Internal(format!("could not start the USB thread ({e})")))?;
    rx.recv()
        .map_err(|_| Error::Internal("the USB thread stopped unexpectedly".into()))?
        .map_err(|e| Error::Device(format!("USB access is not available ({e})")))
}

#[derive(Default)]
pub struct Hid {
    api: Mutex<Option<HidApi>>,
}

impl Hid {
    fn with<T>(&self, f: impl FnOnce(&mut HidApi) -> Result<T>) -> Result<T> {
        let mut guard = self.api.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(create_api()?);
        }
        f(guard.as_mut().expect("initialised above"))
    }

    /// Every interface currently present for this USB vendor and product id.
    pub fn enumerate(&self, vid: u16, pid: u16) -> Result<Vec<Interface>> {
        self.with(|api| {
            api.refresh_devices()
                .map_err(|e| Error::Io(e.to_string()))?;
            Ok(api
                .device_list()
                .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
                .map(|d| Interface {
                    path: d.path().to_owned(),
                    usage_page: d.usage_page(),
                    usage: d.usage(),
                    product: d.product_string().map(str::to_owned),
                })
                .collect())
        })
    }

    pub fn open(&self, path: &CStr) -> Result<HidDevice> {
        self.with(|api| {
            api.open_path(path)
                .map_err(|e| Error::Io(format!("could not open the keyboard ({e})")))
        })
    }
}
