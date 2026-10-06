//! GCC and process execution inside an existing Linux environment or on the host.

use crate::Result;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use wslc_runtime::process::{Output, Runner};

pub fn execute(
    root: &Path,
    program: &Path,
    arguments: &[OsString],
    seconds: u64,
) -> Result<Output> {
    Runner { root }.run(program, arguments, Duration::from_secs(seconds))
}

pub fn checked(root: &Path, program: &Path, arguments: &[OsString]) -> Result<String> {
    let result = execute(root, program, arguments, 60)?;
    if result.code != 0 || !result.stderr.is_empty() {
        return Err(format!(
            "{} {arguments:?}: exit {}\n{}",
            program.display(),
            result.code,
            result.stderr
        ));
    }

    Ok(result.stdout)
}

pub fn assemble(root: &Path, assembly: &Path, executable: &Path) -> Result<()> {
    checked(
        root,
        Path::new("gcc"),
        &[
            "-Wl,--fatal-warnings".into(),
            assembly.as_os_str().to_owned(),
            "-o".into(),
            executable.as_os_str().to_owned(),
        ],
    )?;

    Ok(())
}
