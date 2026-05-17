//! `localasr extract <folder>` — generate a domain-specific terms database
//! by sending file chunks through the configured editor endpoint with a
//! glossary-extraction prompt.

pub mod chunker;
pub mod scanner;
// extractor, merger, run() land in later tasks.
