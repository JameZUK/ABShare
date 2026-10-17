//! ABShare core: pure logic with no I/O (SPEC D6).
//!
//! Hosts (the server, tools, tests) feed this crate numbers and bytes and act on what it
//! returns. Nothing here opens files, sockets or processes.

pub mod range;
pub mod saver;
