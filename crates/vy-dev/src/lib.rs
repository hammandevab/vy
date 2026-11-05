//! `vy-dev`: build a page directly in the tool (SPEC §17, M4).
//!
//! [`scaffold::Scaffold`] (`vy-dev new`) writes a page project: a crate on
//! `vy-sdk`, a manifest draft and a `CLAUDE.md` for a page-authoring agent.
#![deny(missing_docs)]

pub mod scaffold;
