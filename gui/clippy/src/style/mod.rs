//! Transcript presentation as one box: the palette derived from config
//! (skin), syntax colors for the file view (syntax), markdown to styled runs
//! (markdown) and a table laid out for the box it is drawn in (table). Four
//! files, one chain: config in, styled runs out.

pub mod markdown;
pub mod skin;
pub mod syntax;
pub mod table;
