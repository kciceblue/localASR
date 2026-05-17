//! `localasr extract <folder>` — generate a domain-specific terms database
//! by sending file chunks through the configured editor endpoint with a
//! glossary-extraction prompt.

pub mod chunker;
pub mod extractor;
pub mod merger;
pub mod scanner;
// run() lands in a later task.
