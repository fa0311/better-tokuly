//! Integration tests of the moments domain against the dev infrastructure
//! (one test binary, so the modules share a database; see `common`).
//!
//! Prompts are sent by calling the job's handler (`Moments::send_prompt`)
//! and events are fed through `Moments::handle_event`, as the workers would.
mod common;
mod days;
mod lifecycle;
mod posting;
mod reactions;
mod viewing;
