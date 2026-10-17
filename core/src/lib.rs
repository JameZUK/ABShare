//! ABShare core: pure logic with no I/O (SPEC D6).
//!
//! Hosts (the server, tools, tests) feed this crate numbers and bytes and act on what it
//! returns. Nothing here opens files, sockets or processes.

pub mod assemble;
pub mod auth;
pub mod cache;
pub mod chapters;
pub mod ffprobe;
pub mod genre;
pub mod history;
pub mod ids;
pub mod metadata;
pub mod metadata_override;
pub mod model;
pub mod progress;
pub mod range;
pub mod saver;
pub mod series;
pub mod text;
