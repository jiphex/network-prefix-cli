//! The library behind the two binaries in this repository.
//!
//! `prefixtool` inspects, splits and carves up prefixes; `fabrictool` sizes
//! the cabling between switches. They share no domain vocabulary at all, but
//! they do share how output looks: the colour rules in [`style`], the JSON
//! writer in [`json`], and the digit grouping in [`num`]. Those existed before
//! there was a second binary and are hand-rolled rather than pulled in as
//! dependencies, so a second copy of each is the thing to avoid.

pub mod carve;
pub mod fabric;
pub mod info;
pub mod json;
pub mod num;
pub mod ops;
pub mod render;
pub mod report;
pub mod style;
pub mod wellknown;
pub mod zones;
