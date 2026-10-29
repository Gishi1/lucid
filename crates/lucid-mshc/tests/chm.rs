//! End-to-end CHM tests against the committed `testdata/demo-help.chm`.
//!
//! The fixture is self-authored (HTML + HHC + HHK, packed with a small MIT-licensed
//! compiler) so no third-party documentation is redistributed. It exercises exactly the
//! parts of the CHM contract Lucid relies on: `#SYSTEM` metadata, an LZX-compressed
//! `MSCompressed` section, an HHC contents tree, an HHK keyword index, relative links,
//! `ms-its:` links and an asset.

use std::path::PathBuf;

use lucid_mshc::chm::is_chm;
use lucid_mshc::{ChmStore, DocSet, Error, LinkTarget, SourceKind};

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

fn open_chm() -> DocSet {
    let mut progress = |_p: lucid_mshc::Progress| {};
    DocSet::open(&testdata("demo-help.chm"), &mut progress).expect("open the CHM fixture")
}

#[test]
fn opens_a_chm_archive() {
    let set = open_chm();
    assert_eq!(set.kind, SourceKind::Chm);
    assert_eq!(set.kind.label(), "CHM archive");
    assert_eq!(set.packages.len(), 1);
    assert_eq!(set.topic_count(), 4);

    let package = &set.packages[0];
    assert_eq!(package.name, "demo-help");
    assert_eq!(package.title.as_deref(), Some("Lucid CHM Demo"));
    assert_eq!(package.locale.as_deref(), Some("en-us"));
    assert!(package.warnings.is_empty(), "{:?}", package.warnings);
}

#[test]
fn reconstructs_the_hhc_contents() {
    let set = open_chm();
    let roots: Vec<&str> = set.toc.iter().map(|node| node.title.as_str()).collect();
    assert_eq!(roots, ["Welcome", "Guides", "Reference"]);

    let guides = &set.toc[1];
    assert!(guides.key.is_none(), "a grouping node has no topic of its own");
    let children: Vec<&str> = guides
        .children
        .iter()
        .map(|node| node.title.as_str())
        .collect();
    assert_eq!(children, ["Installation", "Using Lucid"]);
    assert!(guides.children.iter().all(|child| child.key.is_some()));

    // Every topic is reachable from the contents.
    let mut revealed = 0;
    lucid_mshc::TocNode::walk_forest(&set.toc, &mut |_node, _level| revealed += 1);
    assert_eq!(revealed, 5); // Welcome, Guides, Installation, Using Lucid, Reference
}

#[test]
fn reads_the_hhk_keyword_index() {
    let set = open_chm();
    let keywords: Vec<&str> = set
        .keywords
        .iter()
        .map(|entry| entry.keyword.as_str())
        .collect();
    assert_eq!(
        keywords,
        ["API reference", "installation", "keyboard shortcuts"]
    );
    // Index entries resolve to their topics.
    let install = set.find_by_id("Demo.Install").expect("topic id");
    let matches = set.keyword_matches("install", 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].key, install);
}

#[test]
fn topic_ids_links_and_assets() {
    let set = open_chm();
    let welcome = set.find_by_id("Demo.Welcome").expect("Demo.Welcome");
    let install = set.find_by_id("Demo.Install").expect("Demo.Install");
    let usage = set.find_by_id("Demo.Usage").expect("Demo.Usage");
    let api = set.find_by_id("Demo.Api").expect("Demo.Api");

    assert_eq!(set.topic_title(welcome), "Welcome to Lucid CHM Demo");
    assert!(set
        .topic_html(welcome)
        .expect("topic body")
        .contains("Welcome"));

    // Relative link inside the HHC tree.
    assert_eq!(
        set.resolve(welcome, "guides/install.htm"),
        LinkTarget::Topic(install)
    );
    // `hh.exe`-style and `ms-its:` links.
    assert_eq!(
        set.resolve(welcome, "ms-its:demo-help.chm::/reference/api.htm"),
        LinkTarget::Topic(api)
    );
    assert_eq!(
        set.resolve(
            install,
            "mk:@MSITStore:C:\\docs\\demo-help.chm::/guides/usage.htm"
        ),
        LinkTarget::Topic(usage)
    );
    // An anchor survives the CHM reference.
    assert_eq!(
        set.resolve(install, "../index.htm#top"),
        LinkTarget::Anchor(welcome, "top".to_owned())
    );
    // External and missing links are still classified.
    assert!(matches!(
        set.resolve(welcome, "https://example.com/docs"),
        LinkTarget::External(_)
    ));
    assert!(matches!(
        set.resolve(welcome, "ms-its:other.chm::/nope.htm"),
        LinkTarget::Missing(_)
    ));

    // Assets are read from the compressed section and looked up case-insensitively.
    let logo = set
        .asset(welcome, "assets/logo.png")
        .expect("asset lookup")
        .expect("logo.png");
    assert_eq!(logo.mime, "image/png");
    assert!(logo.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    assert!(set
        .asset(welcome, "ASSETS/LOGO.PNG")
        .expect("case-insensitive lookup")
        .is_some());
}

#[test]
fn store_exposes_system_metadata_and_parts() {
    let store = ChmStore::open(&testdata("demo-help.chm")).expect("open the store");
    let system = store.system();
    assert_eq!(system.title.as_deref(), Some("Lucid CHM Demo"));
    assert_eq!(system.contents_file.as_deref(), Some("demo.hhc"));
    assert_eq!(system.index_file.as_deref(), Some("demo.hhk"));
    assert_eq!(system.default_topic.as_deref(), Some("index.htm"));
    assert_eq!(system.locale.as_deref(), Some("en-us"));
    assert!(system
        .compiled_with
        .as_deref()
        .is_some_and(|value| value.contains("rustchm")));
    assert!(store.warnings().is_empty(), "{:?}", store.warnings());
    assert!(store.manifest().is_some());

    // The `#SYSTEM` part is reachable even though it is not listed as a content part.
    assert!(store.read_path("#SYSTEM").expect("read #SYSTEM").is_some());
    assert!(!store
        .entries()
        .iter()
        .any(|entry| entry.norm.starts_with(['#', '$'])));

    let index = store.find_norm("index.htm").expect("normalized lookup");
    let bytes = store.read(index.index).expect("read a part");
    assert!(String::from_utf8_lossy(&bytes).contains("Welcome"));
    assert!(store.find_reference("/GUIDES/Usage.HTM").is_some());
}

#[test]
fn a_broken_chm_reports_a_clear_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("broken.chm");
    std::fs::write(&path, b"ITSF but not a real archive").expect("write");

    let mut progress = |_p: lucid_mshc::Progress| {};
    let err = DocSet::open(&path, &mut progress).unwrap_err();
    assert!(matches!(err, Error::Chm { .. }), "{err}");
    assert!(err.to_string().contains("not a readable CHM archive"));
}

#[test]
fn the_mshc_fixture_still_opens_as_a_zip() {
    let path = testdata("demo-help.mshc");
    assert!(!is_chm(&path));
    let mut progress = |_p: lucid_mshc::Progress| {};
    let set = DocSet::open(&path, &mut progress).expect("open the MSHC fixture");
    assert_eq!(set.kind, SourceKind::Package);
    assert!(set.topic_count() > 0);
}
