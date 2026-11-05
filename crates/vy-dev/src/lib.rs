//! `vy-dev`: build a page directly in the tool (SPEC §17, M4).
//!
//! - [`scaffold::Scaffold`] (`vy-dev new`) writes a page project: a crate on
//!   `vy-sdk`, a manifest draft and a `CLAUDE.md` for a page-authoring agent.
//! - [`build::build_project`] (`vy-dev build`) runs the pipeline once:
//!   `cargo build` for wasm32, the ABI check (`vy_runtime::check_abi_bytes`,
//!   the code `vy-cli check-abi` runs), the draft against the module
//!   (`vy_publish::build`) and a signed package.
#![deny(missing_docs)]

pub mod build;
pub mod paths;
pub mod project;
pub mod scaffold;

pub use build::{build_project, BuildOutcome};
pub use project::Project;
