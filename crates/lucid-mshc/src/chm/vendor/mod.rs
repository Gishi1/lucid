//! Vendored CHM container reader.
//!
//! This is [libchm](https://github.com/trypsynth/libchm) 0.3.0 (MIT), copied into the
//! crate so Lucid keeps a single dependency graph and its Rust 1.86 MSRV: upstream 0.3.0
//! declares `rust-version = 1.87` and uses `cast_unsigned`/`cast_signed`,
//! `u32::is_multiple_of` and `const Vec::len`. The only changes are module paths
//! (`crate::…` → `super::…`) and those three API substitutions; the parsing logic is
//! untouched. The upstream license is next to this file (`LICENSE.libchm`).
//!
//! Do not call these modules directly: [`super::ChmStore`] is the adapter that turns the
//! reader into Lucid's part-store model.

mod chm;
mod decompress;
mod directory;
mod error;
mod format;
mod lzx;

pub use chm::ChmFile;
pub use directory::{Entry, EntryCategory, EntryKind, EntrySel};
pub use error::{ChmError, LzxError};
