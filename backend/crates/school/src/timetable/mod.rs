//! Timetables (時間割).
//!
//! - `layout`: the periods of a school day; `holidays`: Japanese public
//!   holidays; `resolve`: from a term's grid to the classes of a date (pure).
//! - `terms`, `grid`: the stored term with its weekly grid, one-off changes
//!   and day overrides.
//! - `calendar`: someone's timetable as a viewer may see it, for a range of
//!   dates; `friends`: sharing, copying and free periods in common.
//! - `reminders`: the opt-in reminder before the first class of the day.
//! - `svc`: the RPC handlers.
pub(crate) mod calendar;
mod friends;
mod grid;
mod holidays;
mod layout;
mod reminders;
mod resolve;
mod svc;
pub(crate) mod terms;

pub(crate) use resolve::{ResolvedClass, dates};
