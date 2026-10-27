//! Zip container access with case-insensitive, URL-aware path lookup.
//!
//! Two facts from real packages drive this module:
//!
//! 1. Topics reference assets with a different case than the archive uses
//!    (`<img src="ASSETS/IMAGE.BMP">` vs the stored `assets/image.bmp`) and often with
//!    percent-encoded or `\`-separated paths.
//! 2. A content set can be huge, so parts are read lazily and the reader pool keeps
//!    parallel work (metadata scan, indexing) busy.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

/// One zip entry, pre-indexed for lookups.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Index inside the zip central directory.
    pub index: usize,
    /// Name as stored in the archive.
    pub name: String,
    /// Lookup key: lowercased, `/`-separated, no leading `./`, percent-decoded.
    pub norm: String,
    pub is_dir: bool,
    /// Uncompressed size.
    pub size: u64,
    pub compressed_size: u64,
    pub crc32: u32,
}

impl Entry {
    /// Lowercase file name without directory.
    pub fn base(&self) -> &str {
        match self.norm.rfind('/') {
            Some(idx) => &self.norm[idx + 1..],
            None => &self.norm,
        }
    }

    /// Lowercase extension without the dot.
    pub fn ext(&self) -> &str {
        match self.base().rfind('.') {
            Some(idx) => &self.base()[idx + 1..],
            None => "",
        }
    }

    /// Directory part (lookup space), without trailing slash.
    pub fn dir(&self) -> &str {
        match self.norm.rfind('/') {
            Some(idx) => &self.norm[..idx],
            None => "",
        }
    }
}

/// The zip container of one package.
pub struct ZipStore {
    path: PathBuf,
    entries: Vec<Entry>,
    by_norm: HashMap<String, usize>,
    by_base: HashMap<String, Vec<usize>>,
    /// A small pool of independent archive readers, so several threads can decompress.
    readers: Mutex<Vec<zip::ZipArchive<BufReader<File>>>>,
}

impl std::fmt::Debug for ZipStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipStore")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl ZipStore {
    /// Open a package container, rejecting non-zip inputs with a useful message.
    pub fn open(path: &Path) -> Result<ZipStore> {
        let mut probe = File::open(path).map_err(|e| Error::io(path, e))?;
        let mut magic = [0u8; 4];
        let read = probe.read(&mut magic).unwrap_or(0);
        if read == 4 && &magic == b"MSCF" {
            return Err(Error::Cab {
                path: path.to_path_buf(),
                name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            });
        }
        if read == 4 && &magic[..2] != b"PK" {
            return Err(Error::NotPackage {
                path: path.to_path_buf(),
                reason: "file does not start with a zip signature".to_owned(),
            });
        }
        drop(probe);

        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        let mut archive = zip::ZipArchive::new(BufReader::new(file))
            .map_err(|e| Error::zip(path, e))?;

        let mut entries = Vec::with_capacity(archive.len());
        let mut by_norm = HashMap::with_capacity(archive.len());
        let mut by_base: HashMap<String, Vec<usize>> = HashMap::new();

        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|e| Error::zip(path, e))?;
            let name = file.name().to_owned();
            let norm = normalize_path(&name);
            let entry = Entry {
                index,
                name,
                norm: norm.clone(),
                is_dir: file.is_dir() || norm.ends_with('/'),
                size: file.size(),
                compressed_size: file.compressed_size(),
                crc32: file.crc32(),
            };
            let slot = entries.len();
            by_base.entry(entry.base().to_owned()).or_default().push(slot);
            if !entry.is_dir {
                by_norm.entry(norm).or_insert(slot);
            }
            entries.push(entry);
        }

        let mut readers = Vec::new();
        readers.push(archive);

        Ok(ZipStore {
            path: path.to_path_buf(),
            entries,
            by_norm,
            by_base,
            readers: Mutex::new(readers),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Look up by an already normalized path.
    pub fn find_norm(&self, norm: &str) -> Option<&Entry> {
        self.by_norm.get(norm).map(|idx| &self.entries[*idx])
    }

    /// Look up by a raw path reference from HTML (`src`, `href`), tolerating case,
    /// `\` separators, `%20`, leading `/`, `./`, `../` and a trailing fragment/query.
    pub fn find_reference(&self, raw: &str) -> Option<&Entry> {
        let cleaned = strip_query_and_fragment(raw).trim();
        if cleaned.is_empty() {
            return None;
        }
        let norm = normalize_path(cleaned);
        if let Some(entry) = self.by_norm.get(&norm) {
            return Some(&self.entries[*entry]);
        }
        // Unique basename match (assets sometimes live in a different directory than the
        // reference suggests, but that only works when the name is unambiguous).
        let base = match norm.rfind('/') {
            Some(idx) => &norm[idx + 1..],
            None => norm.as_str(),
        };
        if let Some(candidates) = self.by_base.get(base) {
            let non_dir: Vec<&usize> = candidates
                .iter()
                .filter(|idx| !self.entries[**idx].is_dir)
                .collect();
            if non_dir.len() == 1 {
                return Some(&self.entries[*non_dir[0]]);
            }
        }
        // Suffix match: `images/x.gif` referenced as `x.gif` or vice versa.
        let suffix = format!("/{norm}");
        let mut hits = self
            .entries
            .iter()
            .filter(|e| !e.is_dir && e.norm.ends_with(&suffix));
        if let Some(first) = hits.next() {
            if hits.next().is_none() {
                return Some(first);
            }
        }
        None
    }

    /// Read an entry fully into memory.
    pub fn read(&self, entry_index: usize) -> Result<Vec<u8>> {
        let expected = self
            .entries
            .get(entry_index)
            .map(|e| e.size)
            .unwrap_or(0);
        self.with_archive(|archive| {
            let mut file = archive
                .by_index(entry_index)
                .map_err(|e| Error::zip(&self.path, e))?;
            let mut buf = Vec::with_capacity(expected.min(64 * 1024 * 1024) as usize);
            file.read_to_end(&mut buf)
                .map_err(|e| Error::io(&self.path, e))?;
            Ok(buf)
        })
    }

    /// Read an entry found by reference (see [`ZipStore::find_reference`]).
    pub fn read_reference(&self, raw: &str) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.find_reference(raw) else {
            return Ok(None);
        };
        let index = entry.index;
        self.read(index).map(Some)
    }

    /// Run `f` with an archive reader from the pool (creating one on demand).
    fn with_archive<T>(
        &self,
        f: impl FnOnce(&mut zip::ZipArchive<BufReader<File>>) -> Result<T>,
    ) -> Result<T> {
        let mut reader = {
            let mut pool = self.readers.lock().unwrap_or_else(|e| e.into_inner());
            pool.pop()
        };
        if reader.is_none() {
            let file = File::open(&self.path).map_err(|e| Error::io(&self.path, e))?;
            let archive = zip::ZipArchive::new(BufReader::new(file))
                .map_err(|e| Error::zip(&self.path, e))?;
            reader = Some(archive);
        }
        let mut archive = reader.expect("reader created above");
        let result = f(&mut archive);
        let mut pool = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        if pool.len() < 16 {
            pool.push(archive);
        }
        result
    }
}

/// Strip `?query` and `#fragment` from a path reference.
pub fn strip_query_and_fragment(raw: &str) -> &str {
    let cut = raw
        .find(['?', '#'])
        .unwrap_or(raw.len());
    &raw[..cut]
}

/// Normalize a path reference into lookup space.
///
/// * percent-decoding (`%20` → space),
/// * `\` → `/`,
/// * collapse repeated separators, drop `.` segments, resolve `..` lexically,
/// * strip leading `/`,
/// * lowercase (packages are case-insensitive by convention: the reference case does not
///   match the stored case).
pub fn normalize_path(raw: &str) -> String {
    let decoded = percent_decode(raw);
    let unified = decoded.replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    for segment in unified.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/").to_lowercase()
}

/// Decode `%XX` escapes (invalid sequences are kept verbatim).
pub fn percent_decode(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve `href` against the directory of `base_part` (both in lookup space).
pub fn resolve_relative(base_part: &str, href: &str) -> String {
    let dir = match base_part.rfind('/') {
        Some(idx) => &base_part[..idx],
        None => "",
    };
    let joined = if dir.is_empty() {
        href.to_owned()
    } else {
        format!("{dir}/{href}")
    };
    normalize_path(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_rules() {
        assert_eq!(normalize_path("ASSETS\\IMAGE.BMP"), "assets/image.bmp");
        assert_eq!(normalize_path("./a/./b/../c.htm"), "a/c.htm");
        assert_eq!(normalize_path("/abs/path.htm"), "abs/path.htm");
        assert_eq!(normalize_path("a%20b/c.htm"), "a b/c.htm");
        assert_eq!(normalize_path("a//b///c"), "a/b/c");
        assert_eq!(normalize_path("../outside.htm"), "outside.htm");
    }

    #[test]
    fn percent_decoding_is_forgiving() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }

    #[test]
    fn reference_stripping() {
        assert_eq!(strip_query_and_fragment("a.htm#x"), "a.htm");
        assert_eq!(strip_query_and_fragment("a.htm?x=1#y"), "a.htm");
        assert_eq!(strip_query_and_fragment("a.htm"), "a.htm");
    }

    #[test]
    fn relative_resolution() {
        assert_eq!(resolve_relative("topics/a.htm", "b.htm"), "topics/b.htm");
        assert_eq!(resolve_relative("topics/a.htm", "../img/x.gif"), "img/x.gif");
        assert_eq!(resolve_relative("a.htm", "topics/b.htm"), "topics/b.htm");
    }

    #[test]
    fn missing_file_is_not_a_panic() {
        assert!(ZipStore::open(Path::new("/definitely/not/here.mshc")).is_err());
    }

    #[test]
    fn cab_is_detected() {
        let dir = std::env::temp_dir().join("lucid-mshc-cab-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake.cab");
        std::fs::write(&path, b"MSCF\0\0\0\0rest").unwrap();
        match ZipStore::open(&path) {
            Err(Error::Cab { .. }) => {}
            other => panic!("expected cab error, got {other:?}"),
        }
    }

    #[test]
    fn plain_file_is_rejected_with_context() {
        let dir = std::env::temp_dir().join("lucid-mshc-notzip-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("readme.txt");
        std::fs::write(&path, b"hello").unwrap();
        match ZipStore::open(&path) {
            Err(Error::NotPackage { .. }) => {}
            other => panic!("expected NotPackage, got {other:?}"),
        }
    }
}
