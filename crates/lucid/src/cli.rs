//! Argument parsing and the headless commands (`docs/DESIGN.md` §6).
//!
//! The GUI is the default; everything else is a subcommand so that `lucid book.mshc` or
//! `lucid book.chm` stays a one-keystroke way to open a package.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lucid_mshc::{DocSet, Progress, TopicKey};
use lucid_search::{IndexOptions, SearchOptions, Sort};

use crate::app;
use crate::index_cache;
use crate::load::build_index;

/// Usage text (also shown by `lucid --help`).
pub const USAGE: &str = "\
lucid — read and search MSHC (Microsoft Help Container) and CHM (Compiled HTML Help) documentation

USAGE:
    lucid [<path>] [--topic <id>] [--search <term>] [--f1 <keyword>] [--no-index]
          [--pane <name>] [--theme <name>] [--size <WxH>] [--no-maximize]
          [--screenshot <png>] [--frames <n>]
    lucid info <path>
    lucid toc <path> [--depth N]
    lucid search <path> <query> [--limit N] [--json]
    lucid extract <path> -o <dir> [--force]
    lucid demo <out.mshc> [--force]
    lucid --version | --help

OPTIONS:
    --topic <id>       open the GUI at a topic id (context-sensitive help)
    --search <term>    open the GUI with a query already running
    --f1 <keyword>     resolve an F1 keyword and open that topic
    --no-index         do not build or load the ranked search index
    --limit <n>        maximum results for `search` (default 20)
    --pane <name>      start in this pane: contents | index | search | favorites
    --theme <name>     dark | light (overrides the saved preference)
    --size <WxH>       open the window at exactly this size (windowed, not maximized)
    --no-maximize      start windowed instead of maximized (the default)
    --screenshot <png> write a PNG of the window once the package is loaded, then exit
    --frames <n>       frames to draw before the screenshot (default 6)
    --json             machine-readable output for `search` and `info`
    --depth <n>        maximum TOC depth for `toc`
    -o, --out <dir>    output directory for `extract`
    -f, --force        overwrite existing files

EXIT CODES:
    0 success · 1 I/O or format error · 2 usage error · 3 topic/keyword not found
";

/// Errors that map to exit codes.
pub enum CliError {
    Usage(String),
    Failed(String),
    NotFound(String),
}

impl From<lucid_mshc::Error> for CliError {
    fn from(err: lucid_mshc::Error) -> Self {
        CliError::Failed(err.to_string())
    }
}

impl From<lucid_forge::Error> for CliError {
    fn from(err: lucid_forge::Error) -> Self {
        CliError::Failed(err.to_string())
    }
}

fn value_of(args: &[String], names: &[&str]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if names.contains(&arg.as_str()) {
            return iter.next().cloned();
        }
    }
    None
}

fn flag(args: &[String], names: &[&str]) -> bool {
    args.iter().any(|arg| names.contains(&arg.as_str()))
}

/// Positional arguments, skipping flags and the values they consume.
fn positionals<'a>(args: &'a [String], value_flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if value_flags.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        out.push(arg.as_str());
    }
    out
}

const GUI_VALUE_FLAGS: &[&str] = &[
    "--topic",
    "--search",
    "--f1",
    "--size",
    "--screenshot",
    "--frames",
    "--pane",
    "--theme",
];
const SEARCH_VALUE_FLAGS: &[&str] = &["--limit"];
const TOC_VALUE_FLAGS: &[&str] = &["--depth"];
const EXTRACT_VALUE_FLAGS: &[&str] = &["-o", "--out"];

/// Entry point for the whole CLI.
pub fn dispatch(args: &[String]) -> Result<ExitCode, CliError> {
    if flag(args, &["--version", "-V"]) {
        println!("lucid {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if flag(args, &["--help", "-h"]) {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    match args.first().map(String::as_str) {
        Some("info") => cmd_info(&args[1..]),
        Some("toc") => cmd_toc(&args[1..]),
        Some("search") => cmd_search(&args[1..]),
        Some("extract") => cmd_extract(&args[1..]),
        Some("demo") => cmd_demo(&args[1..]),
        Some("help") => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        // Anything else is either a path (open the GUI) or a mistake.
        _ => cmd_gui(args),
    }
}

/// `lucid [path]` — open the GUI.
fn cmd_gui(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, GUI_VALUE_FLAGS);
    let path = positional
        .first()
        .map(PathBuf::from);
    let topic = value_of(args, &["--topic"]);
    let search = value_of(args, &["--search"]);
    let f1 = value_of(args, &["--f1"]);
    let no_index = flag(args, &["--no-index"]);
    let size = match value_of(args, &["--size"]) {
        Some(value) => Some(parse_size(&value)?),
        None => None,
    };
    let view = app::ViewOptions {
        size,
        // An explicit size means "leave the window alone"; otherwise start maximized.
        maximized: size.is_none() && !flag(args, &["--no-maximize"]),
        screenshot: value_of(args, &["--screenshot"]).map(PathBuf::from),
        frames: value_of(args, &["--frames"])
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(6),
    };
    let pane = match value_of(args, &["--pane"]) {
        Some(name) => Some(app::pane_from_name(&name).ok_or_else(|| {
            CliError::Usage(format!(
                "`{name}` is not a pane (contents, index, search, favorites)"
            ))
        })?),
        None => None,
    };
    let theme = value_of(args, &["--theme"]);
    if let Some(theme) = theme.as_deref() {
        if !theme.eq_ignore_ascii_case("dark") && !theme.eq_ignore_ascii_case("light") {
            return Err(CliError::Usage(format!(
                "`{theme}` is not a theme (dark or light)"
            )));
        }
    }
    let startup = app::Startup { pane, theme };
    app::run(path, topic, search, f1, no_index, view, startup).map_err(CliError::Failed)
}

/// Parse a `WIDTHxHEIGHT` window size.
fn parse_size(value: &str) -> Result<[f32; 2], CliError> {
    let (width, height) = value
        .split_once(['x', 'X', '×'])
        .ok_or_else(|| CliError::Usage(format!("expected a size like 1280x800, got `{value}`")))?;
    let width: f32 = width
        .trim()
        .parse()
        .map_err(|_| CliError::Usage(format!("`{value}` is not a valid size")))?;
    let height: f32 = height
        .trim()
        .parse()
        .map_err(|_| CliError::Usage(format!("`{value}` is not a valid size")))?;
    if width < 320.0 || height < 240.0 {
        return Err(CliError::Usage(format!(
            "`{value}` is too small for the layout (minimum 320x240)"
        )));
    }
    Ok([width, height])
}

fn open(path: &Path) -> Result<DocSet, CliError> {
    let mut progress = |_update: Progress| {};
    DocSet::open(path, &mut progress).map_err(CliError::from)
}

fn cmd_info(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, &[]);
    let Some(path) = positional.first() else {
        return Err(CliError::Usage("info needs a path".to_owned()));
    };
    let set = open(Path::new(path))?;
    let stats = set.stats();
    let json = flag(args, &["--json"]);

    if json {
        let packages: Vec<serde_json::Value> = set
            .packages
            .iter()
            .map(|package| {
                serde_json::json!({
                    "path": package.path.display().to_string(),
                    "name": package.name,
                    "title": package.title,
                    "locale": package.locale,
                    "topics": package.topics.len(),
                    "assets": package.assets.len(),
                    "manifest": package.manifest.as_ref().map(|m| m.part.clone()),
                    "warnings": package.warnings,
                })
            })
            .collect();
        let document = serde_json::json!({
            "path": set.path.display().to_string(),
            "kind": set.kind.label(),
            "packages": packages,
            "topics": stats.topics,
            "assets": stats.assets,
            "toc_nodes": stats.toc_nodes,
            "keywords": stats.keywords,
            "bytes": stats.bytes,
            "fingerprint": set.fingerprint(),
            "locale": set.primary_locale(),
            "warnings": set.warnings,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned())
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{} ({})", set.path.display(), set.kind.label());
    println!(
        "  {} topics · {} assets · {} TOC nodes · {} keywords · {:.1} MiB unpacked",
        stats.topics,
        stats.assets,
        stats.toc_nodes,
        stats.keywords,
        stats.bytes as f64 / (1024.0 * 1024.0)
    );
    println!("  locale {} · fingerprint {}", set.primary_locale().unwrap_or_else(|| "-".to_owned()), set.fingerprint());
    for package in &set.packages {
        println!(
            "  package {} — {} topics, {} assets{}",
            package.name,
            package.topics.len(),
            package.assets.len(),
            match &package.title {
                Some(title) => format!(", title \"{title}\""),
                None => String::new(),
            }
        );
        match &package.manifest {
            Some(manifest) => println!(
                "    manifest {} ({} TOC nodes, {} index entries)",
                manifest.part,
                manifest.toc_size(),
                manifest.index.len()
            ),
            None => println!("    no manifest (contents reconstructed from meta tags)"),
        }
        if !package.duplicate_ids().is_empty() {
            println!("    {} duplicate id(s)", package.duplicate_ids().len());
        }
    }
    let depth = set.toc.iter().map(|node| node.depth()).max().unwrap_or(0);
    println!("  TOC roots {} · max depth {}", set.toc.len(), depth);
    for warning in &set.warnings {
        println!("  warning: {warning}");
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_toc(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, TOC_VALUE_FLAGS);
    let Some(path) = positional.first() else {
        return Err(CliError::Usage("toc needs a path".to_owned()));
    };
    let depth = value_of(args, &["--depth"])
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    let set = open(Path::new(path))?;
    fn walk(
        set: &DocSet,
        nodes: &[lucid_mshc::TocNode<TopicKey>],
        level: usize,
        max_depth: usize,
    ) {
        for node in nodes {
            if level > max_depth {
                continue;
            }
            print!("{}", "  ".repeat(level));
            print!("{}", node.title);
            match node.key {
                Some(key) => {
                    let id = set.topic_id(key);
                    if id.is_empty() {
                        println!("  [{}]", set.topic_part(key));
                    } else {
                        println!("  [{id}]");
                    }
                }
                None => println!("  (group)"),
            }
            walk(set, &node.children, level + 1, max_depth);
        }
    }
    walk(&set, &set.toc, 0, depth);
    Ok(ExitCode::SUCCESS)
}

fn cmd_search(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, SEARCH_VALUE_FLAGS);
    let (Some(path), Some(query)) = (positional.first(), positional.get(1)) else {
        return Err(CliError::Usage(
            "search needs a path and a query: lucid search book.mshc \"phrase\"".to_owned(),
        ));
    };
    let limit = value_of(args, &["--limit"])
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);
    let json = flag(args, &["--json"]);

    let mut progress = |_update: Progress| {};
    let set = DocSet::open(Path::new(path), &mut progress)?;
    // Reuse the persisted index when the package has not changed.
    let started = std::time::Instant::now();
    let (index, from_cache) = match index_for(&set, false) {
        Some(pair) => pair,
        None => (build_index(&set).0, false),
    };
    let setup_ms = started.elapsed().as_millis();
    let options = SearchOptions {
        limit,
        sort: Sort::Relevance,
        snippet: true,
        ..Default::default()
    };
    let results = index.search(query, &options);

    if json {
        let hits: Vec<serde_json::Value> = results
            .hits
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "doc": hit.doc,
                    "score": hit.score,
                    "title": hit.title,
                    "path": hit.path,
                    "snippet": hit.snippet,
                    "fields": hit.matched_fields(),
                })
            })
            .collect();
        let document = serde_json::json!({
            "query": query,
            "total": results.total_matched,
            "took_us": results.took_us,
            "suggestions": results.suggestions.iter().map(|s| s.text.clone()).collect::<Vec<_>>(),
            "hits": hits,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned())
        );
        return Ok(if results.hits.is_empty() {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        });
    }

    let colour = std::io::stdout().is_terminal();
    let (bold, dim, reset) = if colour {
        ("\u{1b}[1m", "\u{1b}[2m", "\u{1b}[0m")
    } else {
        ("", "", "")
    };
    if results.hits.is_empty() {
        let suggestions: Vec<&str> = results
            .suggestions
            .iter()
            .map(|suggestion| suggestion.text.as_str())
            .collect();
        let mut message = format!("no results for `{query}`");
        if !suggestions.is_empty() {
            message.push_str(&format!(" — did you mean: {}", suggestions.join(", ")));
        }
        return Err(CliError::NotFound(message));
    }
    if !flag(args, &["-q", "--quiet"]) {
        println!(
            "{dim}index: {} ({} terms, {setup_ms} ms){reset}",
            if from_cache { "cache hit" } else { "rebuilt" },
            index.term_count()
        );
    }
    println!(
        "{bold}{} result{} in {} µs{reset}",
        results.total_matched,
        if results.total_matched == 1 { "" } else { "s" },
        results.took_us
    );
    for (rank, hit) in results.hits.iter().enumerate() {
        println!(
            "{bold}{:>3}. {}{reset}  {dim}{}{reset}  [{dim}{}{reset}]",
            rank + 1,
            hit.title,
            hit.path,
            hit.matched_fields().join(", ")
        );
        if !hit.snippet.is_empty() {
            println!("      {}", hit.snippet.replace('\n', " "));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_extract(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, EXTRACT_VALUE_FLAGS);
    let Some(path) = positional.first() else {
        return Err(CliError::Usage("extract needs a path".to_owned()));
    };
    let out = value_of(args, &["-o", "--out"]).ok_or_else(|| {
        CliError::Usage("extract needs an output directory (-o <dir>)".to_owned())
    })?;
    let force = flag(args, &["-f", "--force"]);
    let mut progress = |update: Progress| {
        if update.total > 0 {
            eprintln!("{}", update.display());
        }
    };
    let set = DocSet::open(Path::new(path), &mut progress)?;
    let report = lucid_mshc::export::export_book(&set, Path::new(&out), force, &mut progress)?;
    println!(
        "extracted {} file(s), {} asset(s), {} bytes → {}",
        report.files,
        report.assets,
        report.bytes,
        report.out.display()
    );
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_demo(args: &[String]) -> Result<ExitCode, CliError> {
    let positional = positionals(args, &[]);
    let Some(out) = positional.first() else {
        return Err(CliError::Usage("demo needs an output file".to_owned()));
    };
    if Path::new(out).exists() && !flag(args, &["-f", "--force"]) {
        return Err(CliError::Failed(format!(
            "{out} already exists (use --force)"
        )));
    }
    let report = lucid_forge::demo_package(Path::new(out))?;
    println!(
        "wrote {} ({} topics, {} assets, {} bytes)",
        report.out.display(),
        report.topics,
        report.assets,
        report.bytes
    );
    Ok(ExitCode::SUCCESS)
}

/// Build an index for a content set, reusing the cache when it is valid.
///
/// Returns the index and whether it came from the cache (shown by `lucid search` and by the
/// GUI's status bar, so "is the cache working?" is answerable without guessing).
pub fn index_for(set: &DocSet, no_index: bool) -> Option<(lucid_search::Index, bool)> {
    if no_index {
        return None;
    }
    if let Some(index) = index_cache::load(set) {
        return Some((index, true));
    }
    let (index, _) = build_index(set);
    let _ = index_cache::save(set, &index);
    Some((index, false))
}

/// Default index options for a content set: cheap on huge sets, rich on normal books.
pub fn index_options_for(set: &DocSet) -> IndexOptions {
    if set.topic_count() > 20_000 {
        IndexOptions::cheap()
    } else {
        IndexOptions::default()
    }
}

