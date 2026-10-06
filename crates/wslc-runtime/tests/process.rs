use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wslc_runtime::process::{Runner, report};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wslc-process-test-{}-{} 日本語",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn probe(fixture: &Fixture, mode: &str, skip: &[&str]) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "process_probe",
        "--nocapture",
        "--test-threads",
        "1",
    ]);
    for value in skip {
        command.arg("--skip").arg(value);
    }
    command.env("WSLC_PROCESS_TEST_MODE", mode);
    command.env("WSLC_PROCESS_TEST_ROOT", &fixture.0);
    command
}

#[test]
fn process_runner_drains_large_output_and_preserves_status() {
    let fixture = Fixture::new();
    let runner = Runner { root: &fixture.0 };
    let output = runner
        .run_command(probe(&fixture, "io", &[]), Duration::from_secs(10), true)
        .unwrap();
    assert_eq!(output.code, 7);
    assert!(output.stdout.ends_with(&"x".repeat(1_048_576)));
    assert_eq!(output.stderr, "y".repeat(1_048_576));
    let values = ["hello world", "quote\"test", "semi;colon", "日本語"];
    let output = runner
        .run_command(
            probe(&fixture, "echo", &values),
            Duration::from_secs(10),
            true,
        )
        .unwrap();
    assert_eq!(output.code, 0);
    assert!(
        output
            .stdout
            .ends_with(&(serde_json::to_string(&values).unwrap() + "\n"))
    );
}

#[test]
fn process_runner_replaces_invalid_utf8_in_both_streams() {
    let fixture = Fixture::new();
    let output = Runner { root: &fixture.0 }
        .run_command(
            probe(&fixture, "invalid-utf8", &[]),
            Duration::from_secs(10),
            true,
        )
        .unwrap();
    assert_eq!(output.code, 0);
    assert!(output.stdout.ends_with("before�日本語�after"));
    assert_eq!(output.stderr, "error�");
}

#[test]
fn cli_reporting_preserves_platform_exit_status_width() {
    let fixture = Fixture::new();
    let output = Runner { root: &fixture.0 }
        .run_command(
            probe(&fixture, "wide-exit", &[]),
            Duration::from_secs(10),
            true,
        )
        .unwrap();
    // Windows exposes all 32 bits; Unix exposes the low 8 bits of exit(513).
    assert_eq!(output.code, if cfg!(windows) { 513 } else { 1 });
}

#[cfg(windows)]
fn console_processes() -> Vec<u32> {
    let mut processes = vec![0; 16];

    loop {
        // SAFETY: the buffer holds the advertised number of writable process IDs.
        let count = unsafe {
            windows_sys::Win32::System::Console::GetConsoleProcessList(
                processes.as_mut_ptr(),
                processes.len() as u32,
            )
        } as usize;

        if count <= processes.len() {
            processes.truncate(count);
            return processes;
        }

        processes.resize(count, 0);
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires an attached Windows console; run inside a terminal"]
fn streaming_children_keep_the_attached_console() {
    assert!(
        console_processes().contains(&std::process::id()),
        "Run this test inside a Windows terminal"
    );

    let fixture = Fixture::new();
    let mut child = probe(&fixture, "console", &[]);
    child.env("WSLC_PROCESS_TEST_PARENT", std::process::id().to_string());
    let output = Runner { root: &fixture.0 }
        .run_command(child, Duration::from_secs(10), false)
        .unwrap();

    assert_eq!(output.code, 0, "Streaming child lost its console or output");
}

#[test]
fn process_timeout_terminates_descendants_and_releases_pipes() {
    let fixture = Fixture::new();
    let runner = Runner { root: &fixture.0 };
    let started = Instant::now();
    let error = runner
        .run_command(probe(&fixture, "tree", &[]), Duration::from_secs(1), true)
        .unwrap_err();
    assert!(error.contains("Timed out after 1s"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(fixture.0.join("descendant-started").is_file());
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!fixture.0.join("descendant-survived").exists());
}

/// Runs only as an explicitly selected subprocess in the process tests.
#[test]
#[ignore]
fn process_probe() {
    let root = PathBuf::from(std::env::var_os("WSLC_PROCESS_TEST_ROOT").unwrap());
    match std::env::var("WSLC_PROCESS_TEST_MODE").unwrap().as_str() {
        "io" => {
            std::io::stdout().write_all(&vec![b'x'; 1_048_576]).unwrap();
            std::io::stderr().write_all(&vec![b'y'; 1_048_576]).unwrap();
            std::process::exit(7);
        }
        "echo" => {
            let arguments: Vec<_> = std::env::args_os().collect();
            let values: Vec<OsString> = arguments
                .windows(2)
                .filter(|pair| pair[0] == "--skip")
                .map(|pair| pair[1].clone())
                .collect();
            let values: Vec<_> = values.iter().map(|value| value.to_string_lossy()).collect();
            println!("{}", serde_json::to_string(&values).unwrap());
            std::process::exit(0);
        }
        "invalid-utf8" => {
            let mut output = b"before\xff".to_vec();
            output.extend_from_slice("日本語".as_bytes());
            output.extend_from_slice(b"\xfeafter");
            std::io::stdout().write_all(&output).unwrap();
            std::io::stderr().write_all(b"error\xf0\x90").unwrap();
            std::process::exit(0);
        }
        "wide-exit" => {
            let _ = report(Ok(513));
            panic!("wide exit status was not forwarded to the OS");
        }
        #[cfg(windows)]
        "console" => {
            let parent = std::env::var("WSLC_PROCESS_TEST_PARENT")
                .unwrap()
                .parse()
                .unwrap();
            if !console_processes().contains(&parent) {
                std::process::exit(41);
            }

            std::io::stdout().write_all(b"streamed stdout\n").unwrap();
            std::io::stderr().write_all(b"streamed stderr\n").unwrap();
            std::process::exit(0);
        }
        "tree" => {
            let fixture = Fixture(root);
            let mut child = probe(&fixture, "descendant", &[]).spawn().unwrap();
            child.wait().unwrap();
            // The parent and child should both be terminated before returning.
            std::mem::forget(fixture);
        }
        "descendant" => {
            fs::write(root.join("descendant-started"), "started").unwrap();
            std::thread::sleep(Duration::from_secs(3));
            fs::write(root.join("descendant-survived"), "survived").unwrap();
        }
        mode => panic!("Unknown probe: {mode}"),
    }
}
