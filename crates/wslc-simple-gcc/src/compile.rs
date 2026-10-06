//! Validate source selection and build an argument-only container request.

use crate::Result;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use wslc_runtime::container::{Context, Mount, Pull, RunOptions};
#[cfg(windows)]
use wslc_runtime::process::Output;

pub const DEFAULT_IMAGE: &str = "docker.io/library/gcc:16.2.0";
const SCRIPT: &str = include_str!("../assets/compile-run.sh");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Auto,
    C,
    Cpp,
    Assembly,
}

impl Language {
    fn infer(path: &Path) -> Result<Self> {
        match path.extension().and_then(|value| value.to_str()) {
            Some("c") => Ok(Self::C),
            Some("C" | "cc" | "cpp" | "cxx" | "c++") => Ok(Self::Cpp),
            Some("s") => Ok(Self::Assembly),
            _ => Err(format!(
                "Cannot infer language for {}; select --language",
                path.display()
            )),
        }
    }

    fn settings(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::C => ("gcc", "c", "c23"),
            Self::Cpp => ("g++", "c++", "c++23"),
            Self::Assembly => ("gcc", "assembler", ""),
            Self::Auto => unreachable!("prepare resolves the language"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub sources: Vec<PathBuf>,
    pub project: Option<PathBuf>,
    pub language: Language,
    pub standard: Option<String>,
    pub compiler_args: Vec<OsString>,
    pub run_args: Vec<OsString>,
    pub image: String,
    pub interactive: bool,
    pub tty: bool,
}

impl Request {
    pub fn new(sources: Vec<PathBuf>) -> Self {
        Self {
            sources,
            project: None,
            language: Language::Auto,
            standard: None,
            compiler_args: Vec::new(),
            run_args: Vec::new(),
            image: DEFAULT_IMAGE.into(),
            interactive: false,
            tty: false,
        }
    }

    pub fn prepare(&self) -> Result<Prepared> {
        let first = self
            .sources
            .first()
            .ok_or("At least one source file is required")?;
        let first = first
            .canonicalize()
            .map_err(|e| format!("{}: {e}", first.display()))?;
        let project = self
            .project
            .as_deref()
            .unwrap_or_else(|| first.parent().unwrap());
        let project = project.canonicalize().map_err(|e| e.to_string())?;
        if !project.is_dir() || self.image.is_empty() {
            return Err("Project must be a directory and image must be nonempty".into());
        }

        let mut sources = Vec::new();
        let mut language = self.language;

        for source in &self.sources {
            let source = source
                .canonicalize()
                .map_err(|e| format!("{}: {e}", source.display()))?;
            if !source.is_file() {
                return Err(format!("Source is not a file: {}", source.display()));
            }

            let relative = source.strip_prefix(&project).map_err(|_| {
                format!("Source is outside project directory: {}", source.display())
            })?;
            if self.language == Language::Auto {
                let inferred = Language::infer(&source)?;
                if language != Language::Auto && language != inferred {
                    return Err("Mixed source languages require separate compilation".into());
                }
                language = inferred;
            }

            sources.push(OsString::from(format!(
                "/src/{}",
                relative.to_string_lossy().replace('\\', "/")
            )));
        }

        let (compiler, gcc_language, default_standard) = language.settings();
        if language == Language::Assembly && self.standard.is_some() {
            return Err("Assembly does not take a C/C++ language standard".into());
        }
        let standard = self.standard.as_deref().unwrap_or(default_standard);
        if language != Language::Assembly && standard.is_empty() {
            return Err("Language standard must be nonempty".into());
        }

        let mut arguments: Vec<OsString> = vec![
            "-c".into(),
            SCRIPT.replace("\r\n", "\n").into(),
            "gcc-runner".into(),
            compiler.into(),
            gcc_language.into(),
            standard.into(),
            sources.len().to_string().into(),
        ];
        arguments.extend(sources);
        arguments.push(self.compiler_args.len().to_string().into());
        arguments.extend_from_slice(&self.compiler_args);
        arguments.extend_from_slice(&self.run_args);

        Ok(Prepared {
            project,
            arguments,
            image: self.image.clone(),
            interactive: self.interactive,
            tty: self.tty,
        })
    }
}

pub struct Prepared {
    pub project: PathBuf,
    pub arguments: Vec<OsString>,
    pub image: String,
    pub interactive: bool,
    pub tty: bool,
}

impl Prepared {
    #[cfg(windows)]
    pub fn run(&self, context: &Context<'_>, timeout: Duration, capture: bool) -> Result<Output> {
        context.run(
            &RunOptions {
                image: self.image.clone(),
                mounts: vec![Mount {
                    host: self.project.clone(),
                    guest: "/src".into(),
                    read_only: true,
                }],
                workdir: "/src".into(),
                entrypoint: "/bin/bash".into(),
                arguments: self.arguments.clone(),
                pull: Pull::Missing,
                interactive: self.interactive,
                tty: self.tty,
            },
            timeout,
            capture,
        )
    }
}
