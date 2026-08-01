//! The settings panel's nested section boxes, one folder each.
//!
//! A section owns its own row-building and whatever state is only its own; the
//! frame in the parent module owns the cursor, the rail, the writes and the
//! shared row vocabulary a section builds with. Each folder carries its own
//! CONTRACT.md.

pub mod agent;
pub mod appearance;
pub mod mcp;
pub mod prompt;
pub mod sessions;
pub mod skills;
