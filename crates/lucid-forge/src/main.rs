#![forbid(unsafe_code)]
//! `lucid-forge` — pack, generate and validate Microsoft Help Container (`.mshc`) packages.
//!
//! The command line is frozen in `docs/DESIGN.md` §5; argument parsing is hand-rolled (no
//! clap). Exit codes: `0` ok, `1` error, `2` usage error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lucid_forge::{build_package, demo_package, outline, validate, BuildOptions};

const HELP: &str = "\
lucid-forge — pack and validate Microsoft Help Container (.mshc) packages

USAGE:
  lucid-forge build <input-dir> -o <out.mshc> [options]
  lucid-forge demo <out.mshc> [--force]
  lucid-forge validate <file.mshc>
  lucid-forge outline <file.mshc>

BUILD OPTIONS:
  -o, --out <file>       package to write (also accepted as a second positional argument)
      --title <text>     package title (default: the output file name)
      --locale <tag>     content locale written to the manifest (default: en-us)
      --root-id <id>     TocParent value that means \"TOC root\" (default: -1)
      --no-manifest      do not write the <name>.msha HelpContentSet part
      --no-opc           do not write [Content_Types].xml / _rels/.rels
      --allow-missing-ids  derive an id from the file name when Microsoft.Help.Id is absent
      --preserve-times   store source timestamps instead of the fixed 2020-01-01 timestamp
      --force            overwrite an existing package

OTHER:
  -h, --help             show this help
  -V, --version          show the version

EXIT CODES:
  0 ok · 1 error (I/O, format, validation) · 2 usage error
";

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    out: Option<PathBuf>,
    title: Option<String>,
    locale: Option<String>,
    root_id: Option<String>,
    no_manifest: bool,
    no_opc: bool,
    allow_missing_ids: bool,
    preserve_times: bool,
    force: bool,
}

impl Args {
    /// The options that were actually given, with the names used in diagnostics.
    fn used_options(&self) -> Vec<&'static str> {
        let mut used = Vec::new();
        if self.out.is_some() {
            used.push("--out");
        }
        if self.title.is_some() {
            used.push("--title");
        }
        if self.locale.is_some() {
            used.push("--locale");
        }
        if self.root_id.is_some() {
            used.push("--root-id");
        }
        if self.no_manifest {
            used.push("--no-manifest");
        }
        if self.no_opc {
            used.push("--no-opc");
        }
        if self.allow_missing_ids {
            used.push("--allow-missing-ids");
        }
        if self.preserve_times {
            used.push("--preserve-times");
        }
        if self.force {
            used.push("--force");
        }
        used
    }

    fn build_options(&self) -> BuildOptions {
        BuildOptions {
            title: self.title.clone(),
            locale: self.locale.clone().unwrap_or_else(|| "en-us".to_owned()),
            root_id: self.root_id.clone().unwrap_or_else(|| "-1".to_owned()),
            write_manifest: !self.no_manifest,
            write_opc: !self.no_opc,
            allow_missing_ids: self.allow_missing_ids,
            preserve_times: self.preserve_times,
            compression: lucid_forge::Compression::default(),
        }
    }
}

enum Parsed {
    Run(Args),
    Help,
    Version,
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse(&argv) {
        Ok(Parsed::Help) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Version) => {
            println!("lucid-forge {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(args)) => run(args),
        Err(message) => usage_error(&message),
    }
}

fn parse(argv: &[String]) -> Result<Parsed, String> {
    let mut args = Args::default();
    let mut only_positional = false;
    let mut index = 0usize;
    while index < argv.len() {
        let argument = argv[index].as_str();
        index += 1;
        if only_positional || !argument.starts_with('-') || argument == "-" {
            args.positional.push(argument.to_owned());
            continue;
        }
        if argument == "--" {
            only_positional = true;
            continue;
        }
        if let Some((flag, value)) = argument.split_once('=') {
            match flag {
                "-o" | "--out" => args.out = Some(PathBuf::from(value)),
                "--title" => args.title = Some(value.to_owned()),
                "--locale" => args.locale = Some(value.to_owned()),
                "--root-id" => args.root_id = Some(value.to_owned()),
                other => return Err(format!("unknown option {other}")),
            }
            continue;
        }
        match argument {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "-o" | "--out" => {
                args.out = Some(PathBuf::from(take_value(argv, &mut index, argument)?))
            }
            "--title" => args.title = Some(take_value(argv, &mut index, argument)?),
            "--locale" => args.locale = Some(take_value(argv, &mut index, argument)?),
            "--root-id" => args.root_id = Some(take_value(argv, &mut index, argument)?),
            "--no-manifest" => args.no_manifest = true,
            "--no-opc" => args.no_opc = true,
            "--allow-missing-ids" => args.allow_missing_ids = true,
            "--preserve-times" => args.preserve_times = true,
            "--force" => args.force = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Parsed::Run(args))
}

fn take_value(argv: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let value = argv
        .get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))?;
    *index += 1;
    if value.is_empty() {
        return Err(format!("{flag} needs a non-empty value"));
    }
    Ok(value)
}

fn run(args: Args) -> ExitCode {
    let Some(command) = args.positional.first().cloned() else {
        return usage_error("no command given");
    };
    let rest: Vec<String> = args.positional[1..].to_vec();
    let allowed: &[&str] = match command.as_str() {
        "build" => &[
            "--out",
            "--title",
            "--locale",
            "--root-id",
            "--no-manifest",
            "--no-opc",
            "--allow-missing-ids",
            "--preserve-times",
            "--force",
        ],
        "demo" => &["--out", "--force"],
        "validate" | "outline" => &[],
        other => return usage_error(&format!("unknown command {other}")),
    };
    let invalid: Vec<&str> = args
        .used_options()
        .into_iter()
        .filter(|name| !allowed.contains(name))
        .collect();
    if !invalid.is_empty() {
        return usage_error(&format!("{command} does not accept {}", invalid.join(", ")));
    }

    match command.as_str() {
        "build" => cmd_build(&args, &rest),
        "demo" => cmd_demo(&args, &rest),
        "validate" => cmd_validate(&rest),
        _ => cmd_outline(&rest),
    }
}

fn cmd_build(args: &Args, rest: &[String]) -> ExitCode {
    let Some(input) = rest.first() else {
        return usage_error("build needs an input directory");
    };
    let out = args.out.clone().or_else(|| rest.get(1).map(PathBuf::from));
    let Some(out) = out else {
        return usage_error("build needs an output package (-o out.mshc)");
    };
    if let Some(code) = refuse_overwrite(&out, args.force) {
        return code;
    }
    match build_package(Path::new(input), &out, &args.build_options()) {
        Ok(report) => report_written(&report),
        Err(error) => fail(&error.to_string()),
    }
}

fn cmd_demo(args: &Args, rest: &[String]) -> ExitCode {
    let out = rest.first().map(PathBuf::from).or_else(|| args.out.clone());
    let Some(out) = out else {
        return usage_error("demo needs an output package: lucid-forge demo <out.mshc>");
    };
    if let Some(code) = refuse_overwrite(&out, args.force) {
        return code;
    }
    match demo_package(&out) {
        Ok(report) => report_written(&report),
        Err(error) => fail(&error.to_string()),
    }
}

fn cmd_validate(rest: &[String]) -> ExitCode {
    let Some(path) = rest.first() else {
        return usage_error("validate needs a package: lucid-forge validate <file.mshc>");
    };
    match validate(Path::new(path)) {
        Ok(report) => {
            if report.ok {
                println!(
                    "{path}: OK — {} topics, {} assets, {} keywords",
                    report.topics, report.assets, report.keywords
                );
            } else {
                println!(
                    "{path}: INVALID — {} error(s), {} warning(s)",
                    report.errors.len(),
                    report.warnings.len()
                );
            }
            for error in &report.errors {
                eprintln!("error: {error}");
            }
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => fail(&error.to_string()),
    }
}

fn cmd_outline(rest: &[String]) -> ExitCode {
    let Some(path) = rest.first() else {
        return usage_error("outline needs a package: lucid-forge outline <file.mshc>");
    };
    match outline(Path::new(path)) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error.to_string()),
    }
}

/// `build` and `demo` never silently overwrite a package the user already has.
fn refuse_overwrite(out: &Path, force: bool) -> Option<ExitCode> {
    if out.exists() && !force {
        eprintln!(
            "error: {} already exists (use --force to overwrite)",
            out.display()
        );
        return Some(ExitCode::from(1));
    }
    None
}

fn report_written(report: &lucid_forge::BuildReport) -> ExitCode {
    println!(
        "lucid-forge: wrote {} — {} topics, {} assets, {} keywords, {} TOC nodes, {}",
        report.out.display(),
        report.topics,
        report.assets,
        report.keywords,
        report.toc_nodes,
        human_bytes(report.bytes)
    );
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    if !report.warnings.is_empty() {
        eprintln!("lucid-forge: {} warning(s)", report.warnings.len());
    }
    ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::from(1)
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    eprintln!();
    eprintln!("usage: lucid-forge build <input-dir> -o <out.mshc> [options]");
    eprintln!("       lucid-forge demo <out.mshc> [--force]");
    eprintln!("       lucid-forge validate <file.mshc>");
    eprintln!("       lucid-forge outline <file.mshc>");
    eprintln!();
    eprintln!("try `lucid-forge --help` for the full reference");
    ExitCode::from(2)
}

fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
