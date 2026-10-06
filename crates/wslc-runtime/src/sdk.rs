//! Windows HTTPS downloads and checksum-verified local dependencies.

use crate::Result;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, c_void};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{NonNull, null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::Networking::WinHttp::*;

pub(crate) fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err("Embedded NUL in Windows argument".into());
    }

    result.push(0);

    Ok(result)
}

pub fn verify(path: &Path, expected: &str) -> Result<()> {
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Expected a SHA-256 hex digest".into());
    }

    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];

    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }

        hash.update(&buffer[..count]);
    }

    let actual = format!("{:x}", hash.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "Checksum mismatch: {} (expected {expected}, got {actual})",
            path.display()
        ));
    }

    Ok(())
}

struct Http(NonNull<c_void>);

impl Http {
    fn new(handle: *mut c_void) -> Result<Self> {
        NonNull::new(handle)
            .map(Self)
            .ok_or_else(|| std::io::Error::last_os_error().to_string())
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        // SAFETY: this guard owns a valid WinHTTP handle and closes it exactly once.
        unsafe {
            WinHttpCloseHandle(self.0.as_ptr());
        }
    }
}

fn success(value: i32) -> Result<()> {
    if value == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

struct Pending(PathBuf);

impl Pending {
    fn new(path: &Path) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut name = path.as_os_str().to_owned();
        name.push(format!(
            ".partial-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self(name.into())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn fetch(host: &str, resource: &str, destination: &Path, digest: &str) -> Result<()> {
    if destination.is_file() {
        return verify(destination, digest);
    }

    fs::create_dir_all(
        destination
            .parent()
            .ok_or("Download destination has no parent")?,
    )
    .map_err(|e| e.to_string())?;
    let pending = Pending::new(destination);
    eprintln!("Downloading https://{host}{resource}");

    let agent = wide(OsStr::new("wslc-runtime/0.2"))?;
    let host = wide(OsStr::new(host))?;
    let verb = wide(OsStr::new("GET"))?;
    let resource = wide(OsStr::new(resource))?;

    // WinHTTP supplies certificate validation, HTTPS redirects and system proxy
    // settings. Each handle is released even on an I/O or validation failure.
    // SAFETY: agent is terminated UTF-16; null pointers select system proxy settings.
    let http = Http::new(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    })?;

    // SAFETY: the session handle is live; all timeout values are valid milliseconds.
    success(unsafe { WinHttpSetTimeouts(http.0.as_ptr(), 30000, 30000, 60000, 60000) })?;

    // SAFETY: session and terminated host remain live; 443 is the HTTPS port.
    let connection = Http::new(unsafe { WinHttpConnect(http.0.as_ptr(), host.as_ptr(), 443, 0) })?;

    // SAFETY: connection and terminated strings are live; nulls use documented defaults.
    let request = Http::new(unsafe {
        WinHttpOpenRequest(
            connection.0.as_ptr(),
            verb.as_ptr(),
            resource.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        )
    })?;

    // SAFETY: request is live; null buffers with zero lengths send no headers/body.
    success(unsafe { WinHttpSendRequest(request.0.as_ptr(), null(), 0, null(), 0, 0, 0) })?;
    // SAFETY: send succeeded and the documented reserved argument is null.
    success(unsafe { WinHttpReceiveResponse(request.0.as_ptr(), null_mut()) })?;

    let mut status = 0u32;
    let mut size = std::mem::size_of_val(&status) as u32;
    // SAFETY: NUMBER writes a u32 into status; size describes that writable buffer.
    success(unsafe {
        WinHttpQueryHeaders(
            request.0.as_ptr(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            std::ptr::from_mut(&mut status).cast(),
            &mut size,
            null_mut(),
        )
    })?;
    if status != 200 {
        return Err(format!("Dependency download returned HTTP {status}"));
    }
    {
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&pending.0)
            .map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 65536];

        loop {
            let mut count = 0;
            // SAFETY: the live request writes at most buffer.len() bytes and a count.
            success(unsafe {
                WinHttpReadData(
                    request.0.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut count,
                )
            })?;
            if count == 0 {
                break;
            }

            file.write_all(&buffer[..count as usize])
                .map_err(|e| e.to_string())?;
        }
    }

    verify(&pending.0, digest)?;
    fs::rename(&pending.0, destination).map_err(|e| e.to_string())?;

    Ok(())
}

/// The archive and C ABI are pinned together.
#[derive(Clone, Debug)]
pub struct Options {
    pub version: String,
    pub sha256: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            version: "3.0.1".into(),
            sha256: "aa051d97078b004cefe5f1ef22c0eec34d917f8802be1784159d3b9c4d916072".into(),
        }
    }
}

pub fn acquire(cache: &Path, options: &Options) -> Result<PathBuf> {
    if !cfg!(target_arch = "x86_64") {
        return Err("This SDK build requires an x64 Windows host".into());
    }

    let version = &options.version;
    if version != "3.0.1" {
        return Err("This runtime implements the Microsoft.WSL.Containers 3.0.1 C ABI".into());
    }

    let base = cache.join("wslc-sdk");
    let archive = base.join(format!("microsoft.wsl.containers.{version}.zip"));
    fetch(
        "api.nuget.org",
        &format!(
            "/v3-flatcontainer/microsoft.wsl.containers/{version}/microsoft.wsl.containers.{version}.nupkg"
        ),
        &archive,
        &options.sha256,
    )?;

    let entry = "runtimes/win-x64/native/wslcsdk.dll";
    let dll = base.join(version).join(entry);
    let mut zip = zip::ZipArchive::new(File::open(&archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut contents = Vec::new();
    zip.by_name(entry)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut contents)
        .map_err(|e| e.to_string())?;

    // Only this known entry is extracted. Cached bytes are checked against the archive.
    if fs::read(&dll).ok().as_deref() != Some(contents.as_slice()) {
        fs::create_dir_all(dll.parent().unwrap()).map_err(|e| e.to_string())?;
        let pending = Pending::new(&dll);
        fs::write(&pending.0, contents).map_err(|e| e.to_string())?;
        fs::rename(&pending.0, &dll).map_err(|e| e.to_string())?;
    }

    Ok(dll)
}
