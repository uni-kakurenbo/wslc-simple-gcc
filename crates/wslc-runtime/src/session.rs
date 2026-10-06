//! The cache lock, loaded SDK and owned session all have scoped lifetimes.

use crate::Result;
use crate::sdk::wide;
use std::ffi::{CStr, OsStr, c_void};
use std::fs::{self, File};
use std::path::Path;
use std::ptr::{NonNull, null_mut};
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::System::LibraryLoader::*;

pub fn lock(cache: &Path) -> Result<File> {
    let path = cache.join("session.lock");
    fs::create_dir_all(cache).map_err(|e| e.to_string())?;

    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("Open checkout lock {}: {e}", path.display()))?;

    file.try_lock().map_err(|e| {
        format!(
            "Cannot lock {} (another session may be using this cache): {e}",
            path.display()
        )
    })?;

    Ok(file)
}

// Microsoft.WSL.Containers 3.0.1 include/wslcsdk.h declares opaque settings of
// size 72 and alignment 8; STDAPI uses the Windows system calling convention.
#[repr(C, align(8))]
struct Settings([u8; 72]);
type Init = unsafe extern "system" fn(*const u16, *const u16, *mut Settings) -> i32;
type Memory = unsafe extern "system" fn(*mut Settings, u32) -> i32;
type Create = unsafe extern "system" fn(*mut Settings, *mut *mut c_void, *mut *mut u16) -> i32;
type Close = unsafe extern "system" fn(*mut c_void) -> i32;

struct Library(HMODULE);

struct Com;

impl Com {
    fn initialize() -> Result<Self> {
        hresult(
            // SAFETY: COM is initialized on this thread without a reserved argument.
            unsafe { CoInitializeEx(null_mut(), COINIT_MULTITHREADED as u32) },
            "Initialize COM",
        )?;

        Ok(Self)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: initialize succeeded and this guard is dropped on the same thread.
        unsafe {
            CoUninitialize();
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: load owns this module handle; Api keeps it live until all calls end.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

pub struct Api {
    _library: Library,
    _com: Com,
    init: Init,
    memory: Memory,
    create: Create,
    terminate: Close,
    release: Close,
}

fn hresult(code: i32, operation: &str) -> Result<()> {
    if code < 0 {
        Err(format!("{operation} failed (0x{:08X})", code as u32))
    } else {
        Ok(())
    }
}

impl Api {
    pub fn load(path: &Path) -> Result<Self> {
        let com = Com::initialize()?;
        let path = wide(path.as_os_str())?;

        // SAFETY: path is NUL-terminated UTF-16 and remains live throughout the call.
        let handle = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle.is_null() {
            return Err(format!(
                "Load WSLC SDK: {}",
                std::io::Error::last_os_error()
            ));
        }

        let library = Library(handle);
        let symbol = |name: &'static CStr| {
            // SAFETY: the module is live and CStr supplies a terminated export name.
            unsafe { GetProcAddress(handle, name.as_ptr().cast()) }
                .ok_or_else(|| format!("Missing WSLC SDK export: {}", name.to_string_lossy()))
        };

        // SAFETY: the pinned SDK header declares this export with the Init signature.
        let init = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Init>(symbol(
                c"WslcInitSessionSettings",
            )?)
        };

        // SAFETY: the pinned SDK header declares this export with the Memory signature.
        let memory = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Memory>(symbol(
                c"WslcSetSessionSettingsMemory",
            )?)
        };

        // SAFETY: the pinned SDK header declares this export with the Create signature.
        let create = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(symbol(
                c"WslcCreateSession",
            )?)
        };

        // SAFETY: the pinned SDK header declares this export with the Close signature.
        let terminate = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(symbol(
                c"WslcTerminateSession",
            )?)
        };

        // SAFETY: the pinned SDK header declares this export with the Close signature.
        let release = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(symbol(
                c"WslcReleaseSession",
            )?)
        };

        Ok(Self {
            _library: library,
            _com: com,
            init,
            memory,
            create,
            terminate,
            release,
        })
    }

    pub fn start<'a>(
        &'a self,
        name: &str,
        storage: &Path,
        memory_mb: u32,
    ) -> Result<OwnedSession<'a>> {
        if name.is_empty() || memory_mb == 0 {
            return Err("Session name and nonzero memory limit are required".into());
        }

        let name_wide = wide(OsStr::new(name))?;
        let storage = wide(storage.as_os_str())?;

        let mut settings = Settings([0; 72]);
        hresult(
            // SAFETY: the terminated strings and ABI-sized settings outlive the call.
            unsafe { (self.init)(name_wide.as_ptr(), storage.as_ptr(), &mut settings) },
            "Initialize WSLC settings",
        )?;
        hresult(
            // SAFETY: init succeeded and settings has the SDK's required size/alignment.
            unsafe { (self.memory)(&mut settings, memory_mb) },
            "Set WSLC memory",
        )?;

        let mut handle = null_mut();
        let mut error = null_mut();
        // SAFETY: initialized settings and both writable output pointers are live.
        let code = unsafe { (self.create)(&mut settings, &mut handle, &mut error) };

        let detail = if !error.is_null() {
            // The API returns a NUL-terminated CoTaskMem-allocated UTF-16 string.
            let mut length = 0;
            // SAFETY: the SDK guarantees a readable UTF-16 buffer through its NUL.
            while unsafe { *error.add(length) } != 0 {
                length += 1;
            }

            // SAFETY: the scan measured the initialized buffer excluding its NUL.
            let detail =
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(error, length) });

            // SAFETY: this output was allocated by COM and is freed exactly once.
            unsafe {
                CoTaskMemFree(error.cast());
            }

            detail
        } else {
            String::new()
        };

        let session = NonNull::new(handle).map(|handle| OwnedSession {
            api: self,
            handle: Some(handle),
            name: name.into(),
        });
        if let Err(error) = hresult(code, "Create WSLC session") {
            drop(session);
            return Err(format!("{error}: {detail}"));
        }

        session.ok_or_else(|| "WSLC SDK returned a null session handle".into())
    }
}

trait SessionApi {
    fn terminate(&self, handle: NonNull<c_void>) -> i32;

    fn release(&self, handle: NonNull<c_void>) -> i32;
}

impl SessionApi for Api {
    fn terminate(&self, handle: NonNull<c_void>) -> i32 {
        // SAFETY: OwnedSession holds a live SDK handle and keeps Api/DLL alive.
        unsafe { (self.terminate)(handle.as_ptr()) }
    }

    fn release(&self, handle: NonNull<c_void>) -> i32 {
        // SAFETY: close removes this owned handle before releasing it exactly once.
        unsafe { (self.release)(handle.as_ptr()) }
    }
}

pub struct OwnedSession<'a> {
    api: &'a dyn SessionApi,
    handle: Option<NonNull<c_void>>,
    name: String,
}

impl OwnedSession<'_> {
    pub fn close(&mut self) -> Result<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };

        let terminated = self.api.terminate(handle);
        let released = self.api.release(handle);

        let mut errors = Vec::new();
        if let Err(error) = hresult(terminated, "Terminate WSLC session") {
            errors.push(error);
        }
        if let Err(error) = hresult(released, "Release WSLC session") {
            errors.push(error);
        }

        if errors.is_empty() {
            eprintln!("Released session {}", self.name);

            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
}

impl Drop for OwnedSession<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<&'static str>>,
        terminate_code: i32,
    }

    impl SessionApi for Fake {
        fn terminate(&self, _: NonNull<c_void>) -> i32 {
            self.calls.borrow_mut().push("terminate");
            self.terminate_code
        }

        fn release(&self, _: NonNull<c_void>) -> i32 {
            self.calls.borrow_mut().push("release");
            0
        }
    }

    #[test]
    fn release_runs_after_termination_failure_and_is_idempotent() {
        let api = Fake {
            calls: RefCell::new(Vec::new()),
            terminate_code: -1,
        };

        let mut session = OwnedSession {
            api: &api,
            handle: Some(NonNull::dangling()),
            name: "test".into(),
        };

        assert!(session.close().unwrap_err().contains("FFFFFFFF"));
        session.close().unwrap();
        drop(session);

        assert_eq!(*api.calls.borrow(), ["terminate", "release"]);
    }

    #[test]
    fn session_is_released_when_the_operation_returns_an_error_or_panics() {
        let api = Fake {
            calls: RefCell::new(Vec::new()),
            terminate_code: 0,
        };

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _session = OwnedSession {
                api: &api,
                handle: Some(NonNull::dangling()),
                name: "test".into(),
            };
            panic!("operation failed");
        }));

        assert!(outcome.is_err());
        assert_eq!(*api.calls.borrow(), ["terminate", "release"]);

        let operation = || -> Result<()> {
            let _session = OwnedSession {
                api: &api,
                handle: Some(NonNull::dangling()),
                name: "test".into(),
            };
            Err("operation failed".into())
        };

        let failed_operation = operation();
        assert!(failed_operation.is_err());
        assert_eq!(
            *api.calls.borrow(),
            ["terminate", "release", "terminate", "release"]
        );
    }

    #[test]
    fn sdk_settings_match_the_header_abi() {
        assert_eq!(std::mem::size_of::<Settings>(), 72);
        assert_eq!(std::mem::align_of::<Settings>(), 8);
    }
}
