//! The periodic queue pass: fills in sessionIds, detects runs that finished, reads their
//! result, applies the transition and launches whatever fits.
//!
//! Moved to `nodal_app::execution::pump` (wave 3c), run through `App::run_pump`/`Execution::
//! kick` instead of `work::init`'s own loop; nothing in this crate calls it at this path
//! anymore.
