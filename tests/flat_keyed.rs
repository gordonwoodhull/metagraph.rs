//! The flat keyed placements (M3 step 5): `hash_index` bound in a plain
//! `implement!` — the plain-self placement (a keyed arrow sourced at an
//! element entity, the map on its own record) and the store-level
//! placement (a keyed arrow sourced at the root, the map on the store, the
//! op self-less). M2's keyed tests are all composed (host-embedded and
//! store-level under fusion); this drives the third self placement, which
//! existed in code but had no test.

use metagraph::implement;

implement! {
    m2_fixtures::shapes::KeyedPlain as KeyedStore {
        nodes: arena,
        tags:  hash_index,
        marks: hash_index,
    }
}

#[test]
fn plain_self_keyed_map_round_trips() {
    let mut s = KeyedStore::new();
    let n0 = s.add_n();
    let n1 = s.add_n();
    let m0 = s.add_m();
    let m1 = s.add_m();
    // Per-self maps: each N holds its own N -> M map.
    s.insert_tags(n0, n0, m0);
    s.insert_tags(n0, n1, m1);
    s.insert_tags(n1, n0, m1);
    assert_eq!(s.tags_by_key(n0, n0), Some(m0));
    assert_eq!(s.tags_by_key(n0, n1), Some(m1));
    assert_eq!(s.tags_by_key(n1, n0), Some(m1));
    // The same key under a different self is a different cell.
    assert_eq!(s.tags_by_key(n1, n1), None);
    assert!(s.validate().is_ok());
}

#[test]
fn store_level_keyed_map_round_trips() {
    let mut s = KeyedStore::new();
    let m0 = s.add_m();
    let m1 = s.add_m();
    let m2 = s.add_m();
    // Root-sourced: exactly one self (the store), so the op drops the self
    // argument entirely.
    s.insert_marks(m0, m0);
    s.insert_marks(m1, m1);
    assert_eq!(s.marks_by_key(m0), Some(m0));
    assert_eq!(s.marks_by_key(m1), Some(m1));
    // An un-inserted key is None.
    assert_eq!(s.marks_by_key(m2), None);
    assert!(s.validate().is_ok());
}

#[test]
#[should_panic(expected = "duplicate key")]
fn duplicate_key_rejected_at_the_insert_call() {
    let mut s = KeyedStore::new();
    let n0 = s.add_n();
    let m0 = s.add_m();
    let m1 = s.add_m();
    s.insert_tags(n0, n0, m0);
    // A keyed arrow is a function: the second value for a live key panics
    // where the op is submitted (check-then-write).
    s.insert_tags(n0, n0, m1);
}
