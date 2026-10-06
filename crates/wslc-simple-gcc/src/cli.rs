use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use wslc_simple_gcc::compile::{Language, Request};

pub const HELP: &str = "\
wslc-simple-gcc [run] SOURCE... [OPTIONS] [-- PROGRAM_ARGUMENTS...]
wslc-simple-gcc doctor [--wslc EXECUTABLE]

  --project-dir DIRECTORY     Read-only project mounted at /src
  --language auto|c|cpp|asm    Default: infer from source extensions
  --standard NAME            Default: c23 or c++23
  --compiler-arg VALUE       Repeat for each compiler/linker argument
  --run-arg VALUE            Repeat for each literal program argument
  --image IMAGE              Default: docker.io/library/gcc:16.2.0
  --cache-dir DIRECTORY      SDK cache and owned session storage
  --wslc EXECUTABLE           Explicit wslc.exe path
  --timeout-seconds SECONDS   Default: 1800
  --interactive              Inherit console input
  --tty                      Allocate a terminal and inherit input
  --help                     Show this help
  --version                  Show the CLI version
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Run,
    Doctor,
    Help,
    Version,
}

pub struct Options {
    pub action: Action,
    pub request: Request,
    pub cache: PathBuf,
    pub executable: Option<PathBuf>,
    pub timeout: Duration,
}

fn default_cache() -> PathBuf {
    if let Some(directory) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(directory).join("wslc-simple-gcc")
    } else {
        std::env::temp_dir().join("wslc-simple-gcc")
    }
}

pub fn parse(arguments: &[OsString]) -> Result<Options, String> {
    let mut args = arguments.iter().peekable();
    let action = match args.peek().and_then(|value| value.to_str()) {
        Some("run") => {
            args.next();
            Action::Run
        }
        Some("doctor") => {
            args.next();
            Action::Doctor
        }
        Some("--help" | "-h") => {
            args.next();
            Action::Help
        }
        Some("--version" | "-V") => {
            args.next();
            Action::Version
        }
        _ => Action::Run,
    };

    let mut options = Options {
        action,
        request: Request::new(Vec::new()),
        cache: default_cache(),
        executable: None,
        timeout: Duration::from_secs(1800),
    };
    let mut seen = BTreeSet::new();

    while let Some(argument) = args.next() {
        let name = argument.to_string_lossy();
        if name == "--" {
            options.request.run_args.extend(args.cloned());
            break;
        }
        if !name.starts_with('-') {
            options.request.sources.push(argument.into());
            continue;
        }

        if !matches!(name.as_ref(), "--compiler-arg" | "--run-arg")
            && !seen.insert(name.to_string())
        {
            return Err(format!("Duplicate option: {name}"));
        }
        if name == "--interactive" {
            options.request.interactive = true;
            continue;
        }
        if name == "--tty" {
            options.request.tty = true;
            continue;
        }

        if !matches!(
            name.as_ref(),
            "--project-dir"
                | "--standard"
                | "--language"
                | "--image"
                | "--compiler-arg"
                | "--run-arg"
                | "--cache-dir"
                | "--wslc"
                | "--timeout-seconds"
        ) {
            return Err(format!("Unknown option: {name}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {name}"))?;
        let text = || {
            value
                .to_str()
                .ok_or_else(|| format!("{name} requires UTF-8 text"))
        };

        match name.as_ref() {
            "--project-dir" => options.request.project = Some(value.into()),
            "--standard" => options.request.standard = Some(text()?.into()),
            "--language" => {
                options.request.language = match text()? {
                    "auto" => Language::Auto,
                    "c" => Language::C,
                    "cpp" => Language::Cpp,
                    "asm" => Language::Assembly,
                    value => return Err(format!("Unknown language: {value}")),
                }
            }
            "--image" => options.request.image = text()?.into(),
            "--compiler-arg" => options.request.compiler_args.push(value.clone()),
            "--run-arg" => options.request.run_args.push(value.clone()),
            "--cache-dir" => options.cache = value.into(),
            "--wslc" => options.executable = Some(value.into()),
            "--timeout-seconds" => {
                let seconds = text()?.parse::<u64>().map_err(|_| "Invalid timeout")?;
                if seconds == 0 {
                    return Err("Timeout must be greater than zero".into());
                }
                options.timeout = Duration::from_secs(seconds);
            }
            _ => unreachable!("option names were validated above"),
        }
    }

    if action == Action::Doctor
        && (!options.request.sources.is_empty() || !options.request.run_args.is_empty())
    {
        return Err("doctor does not take source files or program arguments".into());
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_arguments_and_compiler_flags_keep_their_boundaries() {
        let values: Vec<OsString> = [
            "run",
            "hello.c",
            "--compiler-arg",
            "-O2",
            "--run-arg",
            "",
            "--",
            "--help",
            "quote\" 日本語",
            "$(literal)",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let options = parse(&values).unwrap();
        assert_eq!(options.request.compiler_args, ["-O2"]);
        assert_eq!(
            options.request.run_args,
            ["", "--help", "quote\" 日本語", "$(literal)"]
        );
    }

    #[test]
    fn invalid_options_fail_before_starting_a_session() {
        for values in [
            vec!["hello.c", "--timeout-seconds", "0"],
            vec!["--unknown"],
            vec!["--image"],
            vec!["--tty", "--tty"],
            vec!["doctor", "hello.c"],
        ] {
            let values: Vec<OsString> = values.into_iter().map(Into::into).collect();
            assert!(parse(&values).is_err());
        }
    }
}
