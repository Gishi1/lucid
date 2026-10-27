//! Build options for [`crate::build_package`].

/// How the parts of a package are compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Store every part uncompressed: fastest to write, largest output.
    Stored,
    /// Deflate every part (the default, and what real packages use).
    #[default]
    Deflate,
}

impl Compression {
    /// The `zip` compression method this option maps to.
    pub const fn method(self) -> zip::CompressionMethod {
        match self {
            Compression::Stored => zip::CompressionMethod::Stored,
            Compression::Deflate => zip::CompressionMethod::Deflated,
        }
    }
}

/// Options for [`crate::build_package`].
///
/// The defaults produce a package that both Lucid and Microsoft Help Viewer accept: a
/// `HelpContentSet` manifest written from the topic meta tags, OPC parts, `-1` as the TOC-root
/// sentinel, the `en-us` locale, deflate compression and fixed entry timestamps so that two
/// builds of the same input are byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// `<HelpContentSet Title>`; `None` uses the output file name.
    pub title: Option<String>,
    /// `<HelpContentSet Language>` and the fallback locale of the package.
    pub locale: String,
    /// The `Microsoft.Help.TocParent` value that means "this topic is a TOC root".
    ///
    /// `-1` (the default) and an absent/empty `TocParent` always mean root.
    pub root_id: String,
    /// Write the `<name>.msha` `HelpContentSet` manifest part.
    ///
    /// Turning this off produces the fixture Lucid uses to test meta-tag-only TOC
    /// reconstruction.
    pub write_manifest: bool,
    /// Write `[Content_Types].xml` and `_rels/.rels`.
    pub write_opc: bool,
    /// Derive `Microsoft.Help.Id` from the file name instead of failing when a topic has none.
    pub allow_missing_ids: bool,
    /// Store the source files' modification times instead of the fixed 2020-01-01 timestamp.
    pub preserve_times: bool,
    /// Compression used for every part.
    pub compression: Compression,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            title: None,
            locale: "en-us".to_owned(),
            root_id: "-1".to_owned(),
            write_manifest: true,
            write_opc: true,
            allow_missing_ids: false,
            preserve_times: false,
            compression: Compression::default(),
        }
    }
}

impl BuildOptions {
    /// Does `parent` mean "TOC root" for these options?
    pub fn is_root_parent(&self, parent: &str) -> bool {
        is_root_parent_id(&self.root_id, parent)
    }
}

/// Does `parent` mean "TOC root" when the configured root id is `root_id`?
///
/// True for an empty/absent value, for `root_id` itself and for the literal `-1`, which real
/// packages use as the root sentinel even when another root id is configured.
pub fn is_root_parent_id(root_id: &str, parent: &str) -> bool {
    let parent = parent.trim();
    parent.is_empty() || parent == "-1" || parent.eq_ignore_ascii_case(root_id.trim())
}
