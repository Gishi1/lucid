//! The container behind one package.
//!
//! Lucid reads two containers: the OPC zip of a `.mshc` and the ITSF/LZX archive of a
//! `.chm`. [`Package`](crate::package::Package) is written against one small surface —
//! enumerate content parts, read one by index, look one up by normalized path or by a raw
//! `src`/`href` reference — so this enum adapts [`ZipStore`] and [`ChmStore`] to that
//! surface and keeps the format-specific code in one place.
//!
//! The CHM container additionally carries `#SYSTEM` metadata (title, locale, default
//! topic), which the package layer consults when present.

use std::path::Path;

use crate::chm::{self, ChmStore};
use crate::error::Result;
use crate::manifest::RawManifest;
use crate::zipread::{Entry, ZipStore};

/// One package container.
#[derive(Debug)]
pub enum Store {
    /// A zip archive (`.mshc`).
    Zip(ZipStore),
    /// A compiled HTML Help archive (`.chm`).
    Chm(ChmStore),
}

impl Store {
    /// Open the container that `path` holds, choosing the reader by magic/extension.
    pub fn open(path: &Path) -> Result<Store> {
        if chm::is_chm(path) {
            Ok(Store::Chm(ChmStore::open(path)?))
        } else {
            Ok(Store::Zip(ZipStore::open(path)?))
        }
    }

    /// Content parts (directories and CHM-internal metadata excluded).
    pub fn entries(&self) -> &[Entry] {
        match self {
            Store::Zip(store) => store.entries(),
            Store::Chm(store) => store.entries(),
        }
    }

    /// Read a part by its index in [`Store::entries`].
    pub fn read(&self, entry_index: usize) -> Result<Vec<u8>> {
        match self {
            Store::Zip(store) => store.read(entry_index),
            Store::Chm(store) => store.read(entry_index),
        }
    }

    /// Look up a part by an already normalized path.
    pub fn find_norm(&self, norm: &str) -> Option<&Entry> {
        match self {
            Store::Zip(store) => store.find_norm(norm),
            Store::Chm(store) => store.find_norm(norm),
        }
    }

    /// Look up a part by a raw path reference from HTML.
    pub fn find_reference(&self, raw: &str) -> Option<&Entry> {
        match self {
            Store::Zip(store) => store.find_reference(raw),
            Store::Chm(store) => store.find_reference(raw),
        }
    }

    /// The container's own manifest, when it has one (the HHC/HHK sitemap of a CHM).
    pub fn manifest(&self) -> Option<&RawManifest> {
        match self {
            Store::Zip(_) => None,
            Store::Chm(store) => store.manifest(),
        }
    }

    /// Container title (`Title=` from `#SYSTEM` for CHM).
    pub fn title(&self) -> Option<String> {
        match self {
            Store::Zip(_) => None,
            Store::Chm(store) => store.system().title.clone(),
        }
    }

    /// Container locale (`LCID` from `#SYSTEM` for CHM).
    pub fn locale(&self) -> Option<String> {
        match self {
            Store::Zip(_) => None,
            Store::Chm(store) => store.system().locale.clone(),
        }
    }

    /// Default topic path (`Default topic=` from `#SYSTEM` for CHM).
    pub fn default_topic(&self) -> Option<String> {
        match self {
            Store::Zip(_) => None,
            Store::Chm(store) => store.system().default_topic.clone(),
        }
    }

    /// Non-fatal problems found while opening the container.
    pub fn warnings(&self) -> &[String] {
        match self {
            Store::Zip(_) => &[],
            Store::Chm(store) => store.warnings(),
        }
    }

    /// True when the container is a CHM archive.
    pub fn is_chm(&self) -> bool {
        matches!(self, Store::Chm(_))
    }

    /// Path of the underlying file.
    pub fn path(&self) -> &Path {
        match self {
            Store::Zip(store) => store.path(),
            Store::Chm(store) => store.path(),
        }
    }
}
