#![forbid(unsafe_code)]
//! # lucid-mshc
//!
//! Reading side of Lucid (see `docs/DESIGN.md`): opens Microsoft Help Container (`.mshc`)
//! packages, compiled HTML Help (`.chm`) archives and content sets, reconstructs the table
//! of contents and keyword index, resolves links between topics, and turns topic XHTML into
//! a block model that the viewer renders natively (and that the search index consumes as
//! plain text).
//!
//! ```no_run
//! use lucid_mshc::{DocSet, TopicKey};
//!
//! # fn main() -> Result<(), lucid_mshc::Error> {
//! let mut progress = |p: lucid_mshc::Progress| eprintln!("{}", p.display());
//! let set = DocSet::open(std::path::Path::new("testdata/demo-help.mshc"), &mut progress)?;
//! println!("{} topics", set.topic_count());
//! for node in &set.toc {
//!     println!("{}", node.title);
//! }
//! if let Some(topic) = set.find_by_id("Guides.Install") {
//!     let document = set.document(topic)?;
//!     println!("{}\n{}", set.topic_title(topic), document.text);
//! }
//! # Ok(())
//! # }
//! ```

pub mod chm;
pub mod docset;
pub mod error;
pub mod export;
pub mod html;
pub mod manifest;
pub mod meta;
pub mod model;
pub mod package;
pub mod progress;
pub mod store;
pub mod textenc;
pub mod toc;
pub mod zipread;

pub use chm::{ChmStore, ChmSystem};
pub use docset::{
    DocSet, DocSetStats, KeywordEntry, LinkTarget, SourceKind, FORMAT_VERSION,
};
pub use error::{Error, Result};
pub use manifest::RawManifest;
pub use meta::RawMeta;
pub use model::{Block, Cell, CodeBlock, Document, Inline, OutlineEntry, Row, Table};
pub use package::{Asset, AssetBytes, OpcInfo, Package, PackageStats, Topic, TopicKey};
pub use progress::{Progress, ProgressFn, Stage};
pub use store::Store;
pub use toc::TocNode;
pub use zipread::{normalize_path, percent_decode, resolve_relative, ZipStore};
