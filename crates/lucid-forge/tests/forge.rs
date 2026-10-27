//! End-to-end tests: build real packages, validate them, patch parts inside a zip, and drive
//! the CLI binary the way a user would.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use lucid_forge::{
    build_package, demo_files, demo_package, extract_meta_bytes, outline, validate, BuildOptions,
    Error, HELP_NAMESPACE,
};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

// ------------------------------------------------------------------ contract ---

#[test]
fn the_public_types_are_send_sync_and_static() {
    // The application crate converts `lucid_forge::Error` into its own error type and passes
    // reports across threads, so this cannot regress silently.
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<Error>();
    assert_send_sync_static::<lucid_forge::BuildReport>();
    assert_send_sync_static::<lucid_forge::ValidationReport>();
    assert_send_sync_static::<lucid_forge::BuildOptions>();
    assert_send_sync_static::<lucid_forge::Compression>();

    let error: Box<dyn std::error::Error + Send + Sync> = Box::new(Error::MissingId("x".into()));
    assert!(error.to_string().contains("x"));
}

// ---------------------------------------------------------------- fixtures ---

fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("a parent directory")).expect("create directories");
    fs::write(&path, bytes).expect("write the file");
}

#[allow(clippy::too_many_arguments)]
fn topic_html(
    title: &str,
    id: &str,
    parent: &str,
    order: i32,
    keywords: &str,
    f1: &str,
    body: &str,
) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n\
         <title>{title}</title>\n\
         <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\" />\n\
         <meta name=\"Microsoft.Help.Id\" content=\"{id}\" />\n\
         <meta name=\"Microsoft.Help.TocParent\" content=\"{parent}\" />\n\
         <meta name=\"Microsoft.Help.TocOrder\" content=\"{order}\" />\n\
         <meta name=\"Microsoft.Help.Keywords\" content=\"{keywords}\" />\n\
         <meta name=\"Microsoft.Help.F1\" content=\"{f1}\" />\n\
         <meta name=\"Microsoft.Help.Locale\" content=\"en-us\" />\n\
         </head>\n<body>\n<div class=\"title\">{title}</div>\n{body}\n</body>\n</html>\n"
    )
}

/// Three topics (a three-level TOC) plus one asset:
///
/// ```text
/// Alpha (root, order 2)
///   Beta (order 1)      — has an F1 keyword that repeats one of Alpha's
///     Gamma (order 1)
/// ```
fn three_topic_input(root: &Path) {
    write_file(
        root,
        "topics/alpha.htm",
        topic_html("Alpha", "A", "-1", 2, "alpha, shared", "", "<p>Alpha</p>").as_bytes(),
    );
    write_file(
        root,
        "topics/beta.htm",
        topic_html("Beta", "B", "A", 1, "beta", "shared", "<p>Beta</p>").as_bytes(),
    );
    write_file(
        root,
        "topics/gamma.html",
        topic_html("Gamma", "C", "B", 1, "gamma", "", "<p>Gamma</p>").as_bytes(),
    );
    write_file(root, "assets/dot.gif", b"GIF89a\x01\x00\x01\x00\x00\x00");
}

fn fixture_options() -> BuildOptions {
    BuildOptions {
        title: Some("Fixture".to_owned()),
        ..BuildOptions::default()
    }
}

fn zip_names(path: &Path) -> Vec<String> {
    let archive = ZipArchive::new(File::open(path).expect("open the package")).expect("zip");
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    names
}

fn zip_text(path: &Path, name: &str) -> String {
    let mut archive = ZipArchive::new(File::open(path).expect("open the package")).expect("zip");
    let mut entry = archive.by_name(name).expect("the named part");
    let mut text = String::new();
    entry.read_to_string(&mut text).expect("read the part");
    text
}

/// Rewrite a package with some parts replaced (the validator has to cope with hand-edited
/// packages, so the tests produce them the same way).
fn patch_package(source: &Path, target: &Path, edits: &[(&str, String)]) {
    let mut archive = ZipArchive::new(File::open(source).expect("open the source")).expect("zip");
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read the entry");
        entries.push((name, bytes));
    }

    let mut writer = zip::ZipWriter::new(File::create(target).expect("create the target"));
    let options = SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
    for (name, mut bytes) in entries {
        if let Some((_, replacement)) = edits.iter().find(|(edited, _)| *edited == name) {
            bytes = replacement.clone().into_bytes();
        }
        writer.start_file(name, options).expect("start the part");
        writer.write_all(&bytes).expect("write the part");
    }
    writer.finish().expect("finish the archive");
}

fn forge() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lucid-forge"))
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ------------------------------------------------------------- demo package ---

#[test]
fn the_demo_package_validates_cleanly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("demo.mshc");
    let report = demo_package(&out).expect("build the demo");
    assert!(report.topics >= 14, "{} topics", report.topics);
    assert_eq!(report.assets, 2, "logo.bmp and theme.css");
    assert!(report.keywords >= 20, "{} keywords", report.keywords);
    assert_eq!(report.toc_nodes, report.topics);
    assert!(report.bytes > 0);

    let checked = validate(&out).expect("validate the demo");
    assert!(
        checked.ok,
        "the demo must validate cleanly: {:?}",
        checked.errors
    );
    assert!(checked.errors.is_empty());
    assert_eq!(checked.topics, report.topics);
    assert_eq!(checked.assets, report.assets);
    assert_eq!(checked.keywords, report.keywords);
    assert!(
        checked
            .warnings
            .iter()
            .any(|warning| warning.contains("TocParent")),
        "the demo's orphan topic must be reported: {:?}",
        checked.warnings
    );
}

#[test]
fn the_demo_outline_shows_the_expected_nesting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("demo.mshc");
    demo_package(&out).expect("build the demo");
    let text = outline(&out).expect("outline the demo");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("TOC ("), "{text}");
    assert!(lines[0].ends_with(" roots)"), "{text}");

    let roots: Vec<&&str> = lines[1..]
        .iter()
        .filter(|line| !line.starts_with(' '))
        .collect();
    let root_titles: Vec<String> = roots
        .iter()
        .map(|line| {
            line.split_once("  [")
                .map(|(title, _)| title.to_owned())
                .unwrap_or_else(|| (**line).to_owned())
        })
        .collect();
    assert_eq!(
        root_titles,
        vec![
            "Introduction to Lucid",
            "Guides",
            "Reference",
            "Unlisted notes"
        ],
        "TocOrder beats the alphabet: {text}"
    );

    let guides = lines
        .iter()
        .position(|line| line.starts_with("Guides  ["))
        .expect("Guides is a root");
    assert!(lines[guides + 1].starts_with("  Getting started  ["));
    assert!(lines[guides + 2].starts_with("    Installing Lucid  ["));
    assert!(lines[guides + 2].contains("[Lucid.Guides.GettingStarted.Install]"));
    // The orphan keeps its place at the root, exactly as the warning says.
    assert!(roots
        .last()
        .is_some_and(|line| line.contains("Unlisted notes")));
}

#[test]
fn the_demo_assets_are_lowercase_and_referenced_uppercase() {
    let files = demo_files();
    assert!(files.iter().any(|(name, _)| name == "assets/logo.bmp"));
    assert!(files.iter().any(|(name, _)| name == "assets/theme.css"));

    let (_, logo) = files
        .iter()
        .find(|(name, _)| name == "assets/logo.bmp")
        .expect("the logo");
    assert_eq!(&logo[..2], b"BM");
    assert_eq!(logo.len(), 54 + 320 * 3 * 120, "a hand-encoded 24-bit BMP");

    let (_, reader) = files
        .iter()
        .find(|(name, _)| name == "topics/guides/reader/index.htm")
        .expect("the reader topic");
    let html = String::from_utf8(reader.clone()).expect("UTF-8");
    assert!(
        html.contains("<img src=\"ASSETS/LOGO.BMP\""),
        "uppercase asset reference"
    );
    assert!(html.contains("href=\"ASSETS/THEME.CSS\""));
}

#[test]
fn the_demo_contains_a_windows_1252_topic() {
    let files = demo_files();
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| name == "topics/reference/locales.htm")
        .expect("the locales topic");
    assert!(
        std::str::from_utf8(bytes).is_err(),
        "the topic must not be plain UTF-8"
    );
    let head = String::from_utf8_lossy(&bytes[..256]).to_ascii_lowercase();
    assert!(head.contains("charset=windows-1252"), "{head}");
    let meta = extract_meta_bytes(bytes);
    assert_eq!(meta.charset.as_deref(), Some("windows-1252"));
    let description = meta.description.unwrap_or_default();
    assert!(description.contains('€'), "{description}");
    assert!(description.contains('é'), "{description}");
}

// ------------------------------------------------------------- build_package ---

#[test]
fn a_three_topic_build_has_the_documented_parts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);
    let out = dir.path().join("fixture.mshc");

    let report = build_package(&input, &out, &fixture_options()).expect("build");
    assert_eq!(report.topics, 3);
    assert_eq!(report.assets, 1);
    assert_eq!(report.toc_nodes, 3);
    assert_eq!(report.keywords, 5, "alpha, shared, beta, shared(F1), gamma");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let names = zip_names(&out);
    assert_eq!(
        names,
        vec![
            "fixture.msha",
            "[Content_Types].xml",
            "_rels/.rels",
            "topics/alpha.htm",
            "topics/beta.htm",
            "topics/gamma.html",
            "assets/dot.gif",
        ],
        "the entry order is part of the contract"
    );

    let manifest = zip_text(&out, "fixture.msha");
    assert!(manifest.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"));
    assert!(manifest.contains(&format!("xmlns=\"{HELP_NAMESPACE}\"")));
    assert!(manifest.contains("Language=\"en-us\" Title=\"Fixture\""));

    let document = roxmltree::Document::parse(&manifest).expect("the manifest is well-formed XML");
    let nodes: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name((HELP_NAMESPACE, "HelpTOCNode")))
        .collect();
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0].attribute("Id"), Some("A"));
    assert_eq!(nodes[0].attribute("Url"), Some("topics/alpha.htm"));
    let beta = nodes[0]
        .children()
        .find(|node| node.is_element())
        .expect("Beta nests below Alpha");
    assert_eq!(beta.attribute("Id"), Some("B"));
    assert_eq!(beta.attribute("Title"), Some("Beta"));
    let gamma = beta
        .children()
        .find(|node| node.is_element())
        .expect("Gamma nests below Beta");
    assert_eq!(gamma.attribute("Id"), Some("C"));
    assert!(gamma.children().all(|node| !node.is_element()));

    let entries: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name((HELP_NAMESPACE, "HelpIndexEntry")))
        .collect();
    assert_eq!(entries.len(), report.keywords);
    let keywords: Vec<&str> = entries
        .iter()
        .filter_map(|node| node.attribute("Keyword"))
        .collect();
    assert!(keywords.contains(&"shared"));
    assert!(keywords.contains(&"gamma"));

    let content_types = zip_text(&out, "[Content_Types].xml");
    assert!(content_types.contains("<Default Extension=\"htm\" ContentType=\"text/html\" />"));
    assert!(content_types.contains("<Default Extension=\"html\" ContentType=\"text/html\" />"));
    assert!(content_types.contains("<Default Extension=\"gif\" ContentType=\"image/gif\" />"));
    assert!(content_types.contains(
        "<Override PartName=\"/fixture.msha\" ContentType=\"application/vnd.ms-help.contentset+xml\" />"
    ));
    assert!(zip_text(&out, "_rels/.rels")
        .contains("Type=\"http://schemas.microsoft.com/help/2010/06/msha\""));

    // The input tree is copied verbatim, topics and assets alike.
    let source = fs::read_to_string(input.join("topics/alpha.htm")).expect("read the source");
    assert_eq!(zip_text(&out, "topics/alpha.htm"), source);
    let checked = validate(&out).expect("validate");
    assert!(checked.ok, "{:?}", checked.errors);
    assert_eq!(checked.topics, 3);
    assert_eq!(checked.assets, 1);
    assert_eq!(checked.keywords, report.keywords);
}

#[test]
fn rebuilding_is_byte_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);
    let out = dir.path().join("fixture.mshc");

    let first_report = build_package(&input, &out, &fixture_options()).expect("first build");
    let first = fs::read(&out).expect("read the first package");
    let second_report = build_package(&input, &out, &fixture_options()).expect("second build");
    let second = fs::read(&out).expect("read the second package");

    assert_eq!(first, second, "two builds of one input must not differ");
    assert_eq!(first_report.bytes, second_report.bytes);
    assert_eq!(first.len() as u64, first_report.bytes);

    // `preserve_times` is deterministic too, as long as the sources do not change.
    let stamped = dir.path().join("stamped.mshc");
    let options = BuildOptions {
        preserve_times: true,
        ..fixture_options()
    };
    build_package(&input, &stamped, &options).expect("stamped build");
    let stamped_first = fs::read(&stamped).expect("read");
    build_package(&input, &stamped, &options).expect("stamped rebuild");
    assert_eq!(stamped_first, fs::read(&stamped).expect("read"));
}

#[test]
fn the_manifest_and_the_opc_parts_can_be_left_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);

    let without_manifest = dir.path().join("no-manifest.mshc");
    build_package(
        &input,
        &without_manifest,
        &BuildOptions {
            write_manifest: false,
            ..fixture_options()
        },
    )
    .expect("build without a manifest");
    let names = zip_names(&without_manifest);
    assert!(
        !names.iter().any(|name| name.ends_with(".msha")),
        "{names:?}"
    );
    assert!(!names.contains(&"_rels/.rels".to_owned()), "{names:?}");
    assert!(names.contains(&"[Content_Types].xml".to_owned()));
    assert!(!zip_text(&without_manifest, "[Content_Types].xml").contains("<Override"));

    // The meta tags alone still reconstruct the whole TOC.
    let text = outline(&without_manifest).expect("outline");
    assert!(text.contains("Alpha"), "{text}");
    assert!(text.contains("  Beta  [B]"), "{text}");
    assert!(text.contains("    Gamma  [C]"), "{text}");
    let checked = validate(&without_manifest).expect("validate");
    assert!(checked.ok, "{:?}", checked.errors);

    let without_opc = dir.path().join("no-opc.mshc");
    build_package(
        &input,
        &without_opc,
        &BuildOptions {
            write_opc: false,
            ..fixture_options()
        },
    )
    .expect("build without OPC parts");
    let names = zip_names(&without_opc);
    assert!(names.contains(&"fixture.msha".to_owned()), "{names:?}");
    assert!(
        !names.contains(&"[Content_Types].xml".to_owned()),
        "{names:?}"
    );
    assert!(!names.contains(&"_rels/.rels".to_owned()), "{names:?}");
    assert_eq!(
        zip_text(&without_manifest, "topics/beta.htm"),
        zip_text(&without_opc, "topics/beta.htm"),
        "topics are copied verbatim in every variant"
    );
    let checked = validate(&without_opc).expect("validate");
    assert!(checked.ok, "{:?}", checked.errors);
    assert!(checked
        .warnings
        .iter()
        .any(|warning| warning.contains("[Content_Types].xml")));
}

#[test]
fn a_missing_id_is_an_error_unless_ids_may_be_derived() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    write_file(
        &input,
        "topics/no-id.htm",
        b"<html><head><title>No id here</title></head><body><p>Text</p></body></html>",
    );
    let out = dir.path().join("out.mshc");

    let error = build_package(&input, &out, &fixture_options()).expect_err("a missing id fails");
    assert!(
        matches!(&error, Error::MissingId(name) if name == "topics/no-id.htm"),
        "{error:?}"
    );
    assert!(error.to_string().contains("no-id.htm"));

    let report = build_package(
        &input,
        &out,
        &BuildOptions {
            allow_missing_ids: true,
            ..fixture_options()
        },
    )
    .expect("deriving the id is allowed");
    assert_eq!(report.topics, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("derived \"no-id\"")),
        "{:?}",
        report.warnings
    );
    let manifest = zip_text(&out, "fixture.msha");
    assert!(manifest.contains("Id=\"no-id\""), "{manifest}");
    assert!(validate(&out).expect("validate").ok);
}

// --------------------------------------------------------------- validation ---

#[test]
fn validate_catches_a_dangling_manifest_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);
    let out = dir.path().join("fixture.mshc");
    build_package(&input, &out, &fixture_options()).expect("build");

    let manifest = zip_text(&out, "fixture.msha");
    let patched = manifest.replace("Url=\"topics/beta.htm\"", "Url=\"topics/missing.htm\"");
    assert_ne!(manifest, patched);
    let broken = dir.path().join("dangling.mshc");
    patch_package(&out, &broken, &[("fixture.msha", patched)]);

    let report = validate(&broken).expect("validation must not fail on bad content");
    assert!(!report.ok);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("topics/missing.htm")),
        "{:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("beta") && error.contains("not listed")),
        "{:?}",
        report.errors
    );
}

#[test]
fn validate_catches_duplicate_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);
    let out = dir.path().join("fixture.mshc");
    build_package(&input, &out, &fixture_options()).expect("build");

    let beta = zip_text(&out, "topics/beta.htm");
    let patched = beta.replace("content=\"B\"", "content=\"A\"");
    assert_ne!(beta, patched);
    let broken = dir.path().join("duplicate.mshc");
    patch_package(&out, &broken, &[("topics/beta.htm", patched)]);

    let report = validate(&broken).expect("validation must not fail on bad content");
    assert!(!report.ok);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("duplicate") && error.contains("topics/beta.htm")),
        "{:?}",
        report.errors
    );
}

#[test]
fn validate_catches_empty_topics_and_broken_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("input");
    three_topic_input(&input);
    let out = dir.path().join("fixture.mshc");
    build_package(&input, &out, &fixture_options()).expect("build");

    let empty = dir.path().join("empty.mshc");
    patch_package(&out, &empty, &[("topics/gamma.html", String::new())]);
    let report = validate(&empty).expect("validation must not fail on bad content");
    assert!(!report.ok);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("gamma.html") && error.contains("empty")),
        "{:?}",
        report.errors
    );

    let garbage = dir.path().join("garbage.mshc");
    write_file(dir.path(), "garbage.mshc", b"not a zip archive at all");
    let report = validate(&garbage).expect("a non-zip is a report, not a panic");
    assert!(!report.ok);
    assert!(!report.errors.is_empty());

    let cab = dir.path().join("cab.mshc");
    write_file(dir.path(), "cab.mshc", b"MSCF\x00\x00\x00\x00payload");
    let report = validate(&cab).expect("a cab is a report, not a panic");
    assert!(!report.ok);
    assert!(report.errors[0].contains("cab"), "{:?}", report.errors);

    assert!(validate(&dir.path().join("does-not-exist.mshc")).is_err());
}

// ---------------------------------------------------------------------- CLI ---

#[test]
fn the_cli_refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("demo.mshc");

    let first = forge().args(["demo"]).arg(&out).output().expect("run");
    assert!(first.status.success(), "{}", stderr_of(&first));
    assert!(stdout_of(&first).contains("topics"));
    let before = fs::read(&out).expect("read");

    let second = forge().args(["demo"]).arg(&out).output().expect("run");
    assert_eq!(second.status.code(), Some(1));
    assert!(
        stderr_of(&second).contains("--force"),
        "{}",
        stderr_of(&second)
    );
    assert_eq!(fs::read(&out).expect("read"), before, "nothing is written");

    let third = forge()
        .args(["demo"])
        .arg(&out)
        .arg("--force")
        .output()
        .expect("run");
    assert!(third.status.success(), "{}", stderr_of(&third));

    // The same rule applies to `build`.
    let input = dir.path().join("input");
    three_topic_input(&input);
    let package = dir.path().join("built.mshc");
    let build = forge()
        .args(["build"])
        .arg(&input)
        .arg("-o")
        .arg(&package)
        .arg("--title")
        .arg("Fixture")
        .output()
        .expect("run");
    assert!(build.status.success(), "{}", stderr_of(&build));
    assert!(stdout_of(&build).contains("3 topics"));
    let again = forge()
        .args(["build"])
        .arg(&input)
        .arg("-o")
        .arg(&package)
        .output()
        .expect("run");
    assert_eq!(again.status.code(), Some(1));
    let forced = forge()
        .args(["build"])
        .arg(&input)
        .arg("-o")
        .arg(&package)
        .arg("--force")
        .output()
        .expect("run");
    assert!(forced.status.success(), "{}", stderr_of(&forced));
    // Without `--title` the manifest is named after the output file.
    assert!(zip_names(&package).contains(&"built.msha".to_owned()));
}

#[test]
fn the_cli_reports_usage_errors_with_exit_code_two() {
    let version = forge().arg("--version").output().expect("run");
    assert!(version.status.success());
    assert!(stdout_of(&version).contains("lucid-forge"));

    let help = forge().arg("--help").output().expect("run");
    assert!(help.status.success());
    assert!(stdout_of(&help).contains("USAGE"));

    for arguments in [
        vec!["nonsense"],
        vec![],
        vec!["build"],
        vec!["build", "some-dir"],
        vec!["validate"],
        vec!["validate", "a.mshc", "--title", "x"],
        vec!["demo", "a.mshc", "--no-opc"],
    ] {
        let output = forge().args(&arguments).output().expect("run");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} must be a usage error: {}",
            stderr_of(&output)
        );
    }

    let missing = forge()
        .args(["validate", "/nonexistent/package.mshc"])
        .output()
        .expect("run");
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr_of(&missing).contains("error"));
}

#[test]
fn the_cli_validates_and_outlines_a_package() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("demo.mshc");
    demo_package(&out).expect("build the demo");

    let checked = forge().args(["validate"]).arg(&out).output().expect("run");
    assert!(checked.status.success(), "{}", stderr_of(&checked));
    assert!(
        stdout_of(&checked).contains("OK"),
        "{}",
        stdout_of(&checked)
    );
    assert!(stdout_of(&checked).contains("topics"));

    let listed = forge().args(["outline"]).arg(&out).output().expect("run");
    assert!(listed.status.success());
    assert!(stdout_of(&listed).contains("Installing Lucid  [Lucid.Guides.GettingStarted.Install]"));

    let broken = dir.path().join("first-topic.htm");
    fs::write(&broken, b"not a package").expect("write");
    let failure = forge()
        .args(["validate"])
        .arg(&broken)
        .output()
        .expect("run");
    assert_eq!(failure.status.code(), Some(1));
}
