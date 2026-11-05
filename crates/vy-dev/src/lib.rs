//! `vy-dev`: build a page directly in the tool (SPEC §17, M4).
//!
//! - [`scaffold::Scaffold`] (`vy-dev new`) writes a page project: a crate on
//!   `vy-sdk`, a manifest draft and a `CLAUDE.md` for a page-authoring agent.
//! - [`build::build_project`] (`vy-dev build`) runs the pipeline once:
//!   `cargo build` for wasm32, the ABI check (`vy_runtime::check_abi_bytes`,
//!   the code `vy-cli check-abi` runs), the draft against the module
//!   (`vy_publish::build`) and a signed package.
//! - [`serve::start`] (`vy-dev serve`) watches the sources, rebuilds, and
//!   serves two origins on loopback: the page under development at
//!   `vy://localhost:<port>/<name>` and Studio with the dev API at
//!   `vy://localhost:<studio-port>/`. The page cannot reach the API: `net`
//!   is granted to a page's own origin only (SPEC-8.2.3, SPEC-9.5.1), and
//!   the two are on different ports.
//! - [`agent`]: the dev API starts Claude Code headless in the project with
//!   the mission from Studio and turns its `stream-json` output into the
//!   log Studio shows. No Anthropic API client: the CLI is the connection.
//!
//! No async runtime: plain threads with a mutex and a condvar.
#![deny(missing_docs)]

pub mod agent;
pub mod build;
pub mod dev;
pub mod log;
pub mod paths;
pub mod project;
pub mod scaffold;
pub mod serve;

pub use agent::AgentCommand;
pub use build::{build_project, BuildOutcome};
pub use dev::Dev;
pub use project::Project;
pub use serve::{start, Handle, ServeConfig};
