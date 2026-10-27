//! Tolerant HTML handling: tokenizer, mini-DOM and character references.
//!
//! This is deliberately *not* a web engine: it is the smallest thing that can take real
//! MSHC/XHTML topics from any authoring tool and turn them into either metadata
//! ([`crate::meta`]) or a block model ([`crate::model`]).

pub mod arena;
pub mod entities;
pub mod lexer;

pub use arena::{AKind, ANode, Arena, NodeId};
pub use entities::decode_entities;
pub use lexer::{Event, Lexer};
