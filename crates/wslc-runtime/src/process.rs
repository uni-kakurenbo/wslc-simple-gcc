//! Arguments are passed directly, with both output streams drained concurrently.
//! Each invocation owns a process tree and releases it on completion or failure.

use crate::Result;
use std::ffi::{OsStr, OsString};
use std::io::Read;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const NATIVE_TIMEOUT: Duration = Duration::from_secs(120);
pub const CONTAINER_TIMEOUT: Duration = Duration::from_secs(1800);

/// Return from CLI entry points after their scoped resources have been dropped.
/// Preserve wide Windows status codes that stable `ExitCode` cannot represent.
pub fn report(result: Result<i32>) -> ExitCode {
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    match u8::try_from(code) {
        Ok(code) => ExitCode::from(code),
        Err(_) => std::process::exit(code),
    }
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn checked(self, operation: &str) -> Result<String> {
        if self.code != 0 {
            return Err(format!(
                "{operation} failed (exit {}).\n{}\n{}",
                self.code, self.stderr, self.stdout
            ));
        }

        eprint!("{}", self.stderr);

        Ok(self.stdout)
    }
}

pub fn find_program(name: &OsStr) -> Result<PathBuf> {
    let path = Path::new(name);
    let candidates = if path.components().count() > 1 || path.is_absolute() {
        vec![std::path::absolute(path).map_err(|e| e.to_string())?]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(path))
            .collect()
    };

    for candidate in candidates {
        if candidate.is_file() {
            return std::path::absolute(candidate).map_err(|e| e.to_string());
        }

        if cfg!(windows) && candidate.extension().is_none() {
            let executable = candidate.with_extension("exe");
            if executable.is_file() {
                return std::path::absolute(executable).map_err(|e| e.to_string());
            }
        }
    }

    Err(format!("Executable not found: {}", name.to_string_lossy()))
}

#[cfg(windows)]
struct Tree(OwnedHandle);

#[cfg(windows)]
impl Tree {
    fn new() -> Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;

        // The unnamed job is private to this invocation. Closing it kills any
        // remaining descendants, including those which still hold pipe handles.
        // SAFETY: null security attributes and name create a private unnamed job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }

        // SAFETY: CreateJobObjectW returned a valid handle owned by this invocation.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: the job handle is live and the limits pointer matches its size.
        let success = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }

        Ok(job)
    }

    fn attach(&mut self, child: &Child) -> Result<()> {
        // SAFETY: both handles remain live for the duration of the API call.
        let success = unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                self.0.as_raw_handle(),
                child.as_raw_handle(),
            )
        };
        if success == 0 {
            return Err(format!(
                "Could not own process tree: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(())
    }

    fn terminate(&mut self) {
        // SAFETY: this invocation owns the live job and all assigned processes.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}

#[cfg(unix)]
struct Tree(i32);

#[cfg(unix)]
impl Tree {
    fn new() -> Result<Self> {
        Ok(Self(0))
    }

    fn attach(&mut self, child: &Child) -> Result<()> {
        self.0 = child.id() as i32;

        Ok(())
    }

    fn terminate(&mut self) {
        if self.0 != 0 {
            // SAFETY: the child was started in its own process group with this ID.
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
            self.0 = 0;
        }
    }
}

struct OwnedChild {
    child: Child,
    tree: Tree,
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.tree.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reader(stream: impl Read + Send + 'static) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stream = stream;
        stream.read_to_end(&mut bytes)?;

        Ok(bytes)
    })
}

fn collect(handle: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>) -> Result<String> {
    let Some(handle) = handle else {
        return Ok(String::new());
    };

    let bytes = handle
        .join()
        .map_err(|_| "Output reader failed")?
        .map_err(|e| e.to_string())?;

    Ok(String::from_utf8_lossy_owned(bytes))
}

pub struct Runner<'a> {
    pub root: &'a Path,
}

impl Runner<'_> {
    pub fn command(&self, file: &Path, arguments: &[OsString]) -> Command {
        let mut command = Command::new(file);
        command.args(arguments).current_dir(self.root);

        command
    }

    pub fn run(&self, file: &Path, arguments: &[OsString], timeout: Duration) -> Result<Output> {
        self.run_command(self.command(file, arguments), timeout, true)
    }

    pub fn stream(&self, file: &Path, arguments: &[OsString], timeout: Duration) -> Result<i32> {
        Ok(self
            .run_command(self.command(file, arguments), timeout, false)?
            .code)
    }

    pub fn run_command(
        &self,
        command: Command,
        timeout: Duration,
        capture: bool,
    ) -> Result<Output> {
        self.run_with_input(command, timeout, capture, false)
    }

    pub fn run_with_input(
        &self,
        mut command: Command,
        timeout: Duration,
        capture: bool,
        interactive: bool,
    ) -> Result<Output> {
        if timeout.is_zero() {
            return Err("Process timeout must be greater than zero".into());
        }

        command.current_dir(self.root).stdin(if interactive {
            Stdio::inherit()
        } else {
            Stdio::null()
        });

        if capture {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Streaming children inherit the parent's console and output handles.
            if capture && !interactive {
                command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut tree = Tree::new()?;
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Could not start {}: {e}",
                command.get_program().to_string_lossy()
            )
        })?;
        if let Err(error) = tree.attach(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        let mut owned = OwnedChild { child, tree };
        let stdout = owned.child.stdout.take().map(reader);
        let stderr = owned.child.stderr.take().map(reader);

        let started = Instant::now();
        let status = loop {
            if let Some(status) = owned.child.try_wait().map_err(|e| e.to_string())? {
                break Some(status);
            }

            if started.elapsed() >= timeout {
                break None;
            }

            thread::sleep(Duration::from_millis(10));
        };

        owned.tree.terminate();
        let _ = owned.child.wait();

        let stdout = collect(stdout)?;
        let stderr = collect(stderr)?;

        let status = status.ok_or_else(|| {
            format!(
                "Timed out after {}s: {}",
                timeout.as_secs_f64(),
                command.get_program().to_string_lossy()
            )
        })?;

        Ok(Output {
            code: status.code().unwrap_or(1),
            stdout,
            stderr,
        })
    }
}
