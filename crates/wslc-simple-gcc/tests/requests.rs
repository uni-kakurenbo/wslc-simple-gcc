use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use wslc_simple_gcc::compile::Request;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wslc-request-test-{}-{} 日本語",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn source(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, "int main(void) { return 0; }\n").unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn sources_and_literal_arguments_are_separate_from_shell_code() {
    let fixture = Fixture::new();
    let mut request = Request::new(vec![fixture.source("source space.c")]);
    request.run_args = ["", "quote\"test", "$(literal)", "日本語"]
        .map(OsString::from)
        .into();
    request.compiler_args = vec!["-DMESSAGE=\"hello world\"".into()];
    let prepared = request.prepare().unwrap();
    let args = prepared.arguments;
    assert_eq!(args[3..7], ["gcc", "c", "c23", "1"]);
    assert_eq!(args[7], "/src/source space.c");
    assert_eq!(args[8], "1");
    assert_eq!(
        &args[9..],
        [
            "-DMESSAGE=\"hello world\"",
            "",
            "quote\"test",
            "$(literal)",
            "日本語"
        ]
    );
    assert!(!args[1].to_string_lossy().contains("$(literal)"));
}

#[test]
fn invalid_source_selection_fails_before_container_execution() {
    let fixture = Fixture::new();
    let c = fixture.source("main.c");
    let cpp = fixture.source("unit.cpp");
    assert!(
        Request::new(vec![c.clone(), cpp])
            .prepare()
            .err()
            .unwrap()
            .contains("Mixed")
    );
    assert!(Request::new(Vec::new()).prepare().is_err());

    let mut request = Request::new(vec![c]);
    let separate = Fixture::new();
    request.project = Some(separate.0.clone());
    assert!(request.prepare().err().unwrap().contains("outside"));
}
