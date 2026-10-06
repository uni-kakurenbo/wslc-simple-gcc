#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use wslc_runtime::container::{Mount, Pull, RunOptions, SessionOptions, executable, with_session};
use wslc_runtime::process::Runner;
use wslc_runtime::{sdk, session};
use wslc_simple_gcc::compile::Request;

struct Fixture(PathBuf);

impl Fixture {
    fn put(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires Windows WSLC; downloads the verified SDK and GCC image"]
fn real_gcc_requests_and_owned_session_timeout() {
    let root = std::env::temp_dir().join(format!("wslc-gcc-test-{} 日本語", std::process::id()));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let cache = std::env::var_os("WSLC_GCC_TEST_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("wslc-gcc-test-cache"));
    let options = SessionOptions::new(cache.clone());

    let source = fixture.put("hello.c", "#include <stdio.h>\nint main(int argc, char **argv) { for (int i=1; i<argc; ++i) printf(\"<%s>\\n\", argv[i]); return 0; }\n");
    let bad = fixture.put("bad.c", "#error expected_compile_failure\n");
    let exits = fixture.put("exit code.c", "int main(void) { return 37; }\n");
    let assembly = fixture.put("main.s", ".section .rodata\nmessage: .asciz \"assembly: OK\"\n.text\n.globl main\nmain:\n pushq %rbp\n movq %rsp, %rbp\n leaq message(%rip), %rdi\n call puts@PLT\n xorl %eax, %eax\n popq %rbp\n ret\n.section .note.GNU-stack,\"\",@progbits\n");
    let unit = fixture.put(
        "value.c",
        "#include <value.h>\nint value(void) { return 42; }\n",
    );
    fixture.put("include/value.h", "int value(void);\n");
    let main = fixture.put("main.c", "#include <stdio.h>\n#include <string.h>\n#include <value.h>\nint main(void) { if (value()!=42 || strcmp(MESSAGE,\"hello world\")) return 90; FILE *f=fopen(\"must-not-exist\",\"w\"); if (f) { fclose(f); return 91; } puts(\"multiple sources: OK\"); return 0; }\n");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let mut owned_name = String::new();
    with_session(&fixture.0, &options, |context| {
        owned_name = context.session_name().to_owned();
        let mut request = Request::new(vec![source.clone()]);
        request.run_args = [
            "hello world",
            "",
            "quote\"test",
            "semi;colon",
            "$(literal)",
            "日本語",
        ]
        .map(Into::into)
        .into();
        let output = request
            .prepare()?
            .run(context, Duration::from_secs(300), true)?;
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert_eq!(
            output.stdout,
            "<hello world>\n<>\n<quote\"test>\n<semi;colon>\n<$(literal)>\n<日本語>\n"
        );

        let output = Request::new(vec![exits.clone()]).prepare()?.run(
            context,
            Duration::from_secs(60),
            true,
        )?;
        assert_eq!(output.code, 37);
        let output = Request::new(vec![bad.clone()]).prepare()?.run(
            context,
            Duration::from_secs(60),
            true,
        )?;
        assert_eq!(output.code, 1);
        assert!(output.stderr.contains("expected_compile_failure"));

        let output = Request::new(vec![assembly.clone()]).prepare()?.run(
            context,
            Duration::from_secs(60),
            true,
        )?;
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert_eq!(output.stdout, "assembly: OK\n");

        let mut request = Request::new(vec![main.clone(), unit.clone()]);
        request.compiler_args = ["-Iinclude", "-DMESSAGE=\"hello world\"", "-Werror"]
            .map(Into::into)
            .into();
        let output = request
            .prepare()?
            .run(context, Duration::from_secs(60), true)?;
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert_eq!(output.stdout, "multiple sources: OK\n");
        assert!(!fixture.0.join("must-not-exist").exists());

        for (source, standard) in [
            ("hello.c", "c23"),
            ("hello.cpp", "c++23"),
            ("hello.cpp", "c++26"),
        ] {
            let mut request = Request::new(vec![repo.join("examples").join(source)]);
            request.standard = Some(standard.into());
            let output = request
                .prepare()?
                .run(context, Duration::from_secs(60), true)?;
            assert_eq!(output.code, 0, "{}", output.stderr);
            assert!(output.stdout.contains("Hello"));
        }

        let timeout = RunOptions {
            image: request.image.clone(),
            mounts: vec![Mount {
                host: fixture.0.clone(),
                guest: "/src".into(),
                read_only: true,
            }],
            workdir: "/src".into(),
            entrypoint: "/bin/sleep".into(),
            arguments: vec!["30".into()],
            pull: Pull::Never,
            interactive: false,
            tty: false,
        };
        let error = context
            .run(&timeout, Duration::from_secs(2), true)
            .unwrap_err();
        assert!(error.contains("Timed out after 2s"), "{error}");
        Ok(())
    })
    .unwrap();

    let _lock = session::lock(&cache).unwrap();
    let dll = sdk::acquire(&cache, &options.sdk).unwrap();
    let original = fs::read(&dll).unwrap();
    fs::write(&dll, "damaged DLL").unwrap();
    sdk::acquire(&cache, &options.sdk).unwrap();
    assert_eq!(fs::read(dll).unwrap(), original);

    let runner = Runner { root: &fixture.0 };
    let output = runner
        .run(
            &executable(None).unwrap(),
            &["system".into(), "session".into(), "list".into()],
            Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(output.code, 0);
    assert!(!output.stdout.contains(&owned_name), "{}", output.stdout);
}
