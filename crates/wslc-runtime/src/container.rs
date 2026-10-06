//! Requests carry mounts and argument boundaries independently of application policy.

use crate::process::{Output, Runner, find_program};
use crate::{Result, sdk, session};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub cache: PathBuf,
    pub sdk: sdk::Options,
    pub executable: Option<PathBuf>,
    pub memory_mb: u32,
}

impl SessionOptions {
    pub fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            sdk: sdk::Options::default(),
            executable: None,
            memory_mb: 2048,
        }
    }
}

pub fn executable(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return find_program(path.as_os_str());
    }

    if let Ok(path) = find_program(OsStr::new("wslc.exe")) {
        return Ok(path);
    }

    let path = PathBuf::from(std::env::var_os("ProgramFiles").ok_or("ProgramFiles is missing")?)
        .join("WSL/wslc.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err("WSL Containers is required. Run wsl --update.".into())
    }
}

#[derive(Clone, Debug)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: String,
    pub read_only: bool,
}

impl Mount {
    fn argument(&self) -> Result<OsString> {
        let host = self.host.canonicalize().map_err(|e| e.to_string())?;
        let host = host.to_string_lossy();
        let host = host.strip_prefix(r"\\?\").unwrap_or(&host);
        let bytes = host.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || &bytes[1..3] != b":\\"
            || host[2..].contains(':')
            || !self.guest.starts_with('/')
            || self.guest.contains(':')
        {
            return Err(
                "Mounts require local Windows drive paths and absolute Linux destinations".into(),
            );
        }

        Ok(format!(
            "{host}:{}:{}",
            self.guest,
            if self.read_only { "ro" } else { "rw" }
        )
        .into())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Pull {
    Missing,
    Never,
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub image: String,
    pub mounts: Vec<Mount>,
    pub workdir: String,
    pub entrypoint: String,
    pub arguments: Vec<OsString>,
    pub pull: Pull,
    pub interactive: bool,
    pub tty: bool,
}

pub struct Context<'a> {
    root: &'a Path,
    executable: PathBuf,
    name: String,
}

impl Context<'_> {
    pub fn session_name(&self) -> &str {
        &self.name
    }

    pub fn command(&self, options: &RunOptions) -> Result<Command> {
        if options.image.is_empty()
            || !options.workdir.starts_with('/')
            || !options.entrypoint.starts_with('/')
        {
            return Err(
                "Image and absolute container working directory/entrypoint are required".into(),
            );
        }

        let mut args: Vec<OsString> = vec![
            "--session".into(),
            self.name.clone().into(),
            "run".into(),
            "--rm".into(),
            "--pull".into(),
            match options.pull {
                Pull::Missing => "missing",
                Pull::Never => "never",
            }
            .into(),
        ];

        for mount in &options.mounts {
            args.extend(["--volume".into(), mount.argument()?]);
        }
        if options.interactive || options.tty {
            args.push("--interactive".into());
        }
        if options.tty {
            args.push("--tty".into());
        }

        args.extend([
            "--workdir".into(),
            options.workdir.clone().into(),
            "--entrypoint".into(),
            options.entrypoint.clone().into(),
            options.image.clone().into(),
        ]);
        args.extend_from_slice(&options.arguments);

        Ok(Runner { root: self.root }.command(&self.executable, &args))
    }

    pub fn run(&self, options: &RunOptions, timeout: Duration, capture: bool) -> Result<Output> {
        if capture && (options.interactive || options.tty) {
            return Err("Interactive container execution requires streaming output".into());
        }

        Runner { root: self.root }.run_with_input(
            self.command(options)?,
            timeout,
            capture,
            options.interactive || options.tty,
        )
    }
}

/// Keep the SDK, session and cache lock alive until every container operation finishes.
pub fn with_session<T>(
    root: &Path,
    options: &SessionOptions,
    operation: impl FnOnce(&Context<'_>) -> Result<T>,
) -> Result<T> {
    let executable = executable(options.executable.as_deref())?;
    let cache = std::path::absolute(&options.cache).map_err(|e| e.to_string())?;
    let _lock = session::lock(&cache)?;
    let api = session::Api::load(&sdk::acquire(&cache, &options.sdk)?)?;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let name = format!(
        "wslc-runtime-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    eprintln!("Starting owned session {name}");

    let mut owned = api.start(&name, &cache.join("session-storage"), options.memory_mb)?;
    let context = Context {
        root,
        executable,
        name,
    };
    let result = operation(&context);
    let cleanup = owned.close();

    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}\n{cleanup}")),
    }
}
