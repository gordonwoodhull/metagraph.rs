//! Re-exports the fixture crate so `downstream-indirect` can reach its
//! couriers without a direct dependency — the resolver's one rejected
//! configuration (composition through a re-exporting intermediate is not
//! routed; the implement site must depend on every member's declaring
//! crate directly).

pub use m2_fixtures as fixtures;
