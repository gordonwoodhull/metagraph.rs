//! M4 step 8 — the flat store-level keyed carry: a root-sourced keyed
//! arrow (`marks: G -> M, key(N)` bound `hash_index`) no longer gates the
//! store's `Tx` off. The staging call rejects a live key against the
//! borrowed old store and the already-staged inserts (the staged-key
//! probe's semantics, /tmp/m2-probe-staged-key: `Existing` and `Pending`
//! keys never collide — the ref equality compares kinds — and a staged
//! removal frees no key, so re-pointing one is two flushes). The flush
//! carries the map as state — entries whose key and target both survive,
//! both remapped — and replays staged inserts through the backing's
//! check-then-write hook on top of the carry. The survivors-only
//! refinement (a removed key with otherwise-live entries is a
//! `FlushError`) is the keyed-carry step's; a per-record keyed map
//! (KeyedPlain's `tags`) still gates its store's `Tx` off until then.

use metagraph_exper::implement;

implement! {
    m2_fixtures::shapes::KeyedRoot as KeyedStore {
        nodes: arena,
        marks: hash_index,
    }
}

/// n0..n3, m0..m2, entries n0->m0 and n1->m1 (n2, n3 unkeyed).
fn old_store() -> KeyedStore {
    let mut s = KeyedStore::new();
    for _ in 0..4 {
        s.add_n();
    }
    for _ in 0..3 {
        s.add_m();
    }
    s.insert_marks(NId(0), MId(0));
    s.insert_marks(NId(1), MId(1));
    s
}

#[test]
fn carry_filters_and_remaps() {
    let base = old_store();
    assert_eq!(base.marks_by_key(NId(0)), Some(MId(0)));
    assert_eq!(base.marks_by_key(NId(1)), Some(MId(1)));

    // Remove one entry's TARGET (membership follows the removed record)
    // and one unkeyed key record (no entry, no effect).
    let out = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_m(MId(0));
            tx.remove_n(NId(3));
        })
        .expect("flush");

    // n: [0,1,2] kept; m: [1,2] kept. The (n1, m1) entry carried and
    // remapped: m1 compacted to new id 0.
    assert_eq!(out.report.n_map, vec![Some(0), Some(1), Some(2), None]);
    assert_eq!(out.report.m_map, vec![None, Some(0), Some(1)]);
    assert_eq!(out.store.marks_by_key(NId(0)), None, "target removal cleared it");
    assert_eq!(out.store.marks_by_key(NId(1)), Some(MId(0)), "carried + remapped");
    assert_eq!(out.store.marks_by_key(NId(2)), None);
    out.store.validate().expect("validate");
    // Old store untouched.
    assert_eq!(base.marks_by_key(NId(0)), Some(MId(0)));
}

#[test]
fn staged_insert_with_pending_key_and_target() {
    let base = old_store();

    // A wholly staged entry: new key record, new target, keyed insert.
    let out = base
        .edit()
        .tap_flush(|tx| {
            let n4 = tx.add_n();
            let m3 = tx.add_m();
            tx.insert_marks(n4, m3);
        })
        .expect("flush");

    assert_eq!(out.store.n_count(), 5);
    assert_eq!(out.store.m_count(), 4);
    // Kept n0..n3, staged n4 -> new id 4; carried entries untouched.
    assert_eq!(out.store.marks_by_key(NId(0)), Some(MId(0)));
    assert_eq!(out.store.marks_by_key(NId(4)), Some(MId(3)));
    out.store.validate().expect("validate");
}

#[test]
fn staged_insert_with_existing_key_reads_through() {
    let base = old_store();

    // An Existing key with no old entry (n2 is unkeyed): legal at
    // staging (not live), replayed on top of the carry.
    let out = base
        .edit()
        .tap_flush(|tx| {
            let m = tx.add_m();
            tx.insert_marks(NId(2), m);
        })
        .expect("flush");

    assert_eq!(out.store.marks_by_key(NId(2)), Some(MId(3)));
    assert_eq!(out.store.marks_by_key(NId(0)), Some(MId(0)), "carry intact");
    out.store.validate().expect("validate");
}

#[test]
#[should_panic(expected = "exists in the borrowed old store")]
fn live_key_in_old_store_panics_at_staging() {
    let base = old_store();
    let mut tx = base.edit();
    let m = tx.add_m();
    tx.insert_marks(NId(0), m); // n0 keys m0: live
}

#[test]
#[should_panic(expected = "already staged in this transaction")]
fn already_staged_key_panics_at_staging() {
    let base = old_store();
    let mut tx = base.edit();
    let n = tx.add_n();
    let m0 = tx.add_m();
    let m1 = tx.add_m();
    tx.insert_marks(n, m0);
    tx.insert_marks(n, m1); // the same Pending key twice
}

#[test]
fn existing_and_pending_keys_do_not_collide() {
    // The probe's t3: an Existing key and a Pending ref carrying the same
    // NUMBER are different records — the ref equality compares kinds, so
    // the insert with the Existing key does not see the staged one (and
    // n3 has no old entry: not live).
    let base = old_store();
    let mut tx = base.edit();
    let m0 = tx.add_m(); // op 0
    let m1 = tx.add_m(); // op 1
    let _m2 = tx.add_m(); // op 2
    let n = tx.add_n(); // op 3 -> Pending(3)
    tx.insert_marks(n, m0);
    tx.insert_marks(NId(3), m1);
    let out = tx.flush().expect("flush");

    // Staged n lands at new id 4; the pending-keyed entry points at m0'
    // (new id 3), the existing-keyed one at m1' (new id 4).
    assert_eq!(out.store.marks_by_key(NId(4)), Some(MId(3)));
    assert_eq!(out.store.marks_by_key(NId(3)), Some(MId(4)));
    out.store.validate().expect("validate");
}

#[test]
fn keyed_insert_danglers_and_unknown_ids() {
    let base = old_store();

    // A staged insert naming a REMOVED key is NOT a dangler at the insert
    // op: it names the key from a new place, so the survivors-only rule
    // (the keyed-carry probe's pin, step 11) counts it as a LIVE
    // reference and rejects the KEY REMOVAL, attributing the violation
    // to the removal op with the insert listed in `staged_inserts`. The
    // removed key must be UNKEYED in the old store — a live key panics
    // at staging even when its removal is staged (a staged removal frees
    // no key).
    let err = base
        .edit()
        .tap_flush(|tx| {
            let m = tx.add_m(); // op 0
            tx.remove_n(NId(2)); // op 1 (unkeyed in the old store)
            tx.insert_marks(NId(2), m); // op 2: re-names the removed key
        })
        .expect_err("removed key");
    assert_eq!(
        err.violations,
        vec![Violation::KeyRemovedWithLiveEntries {
            op: 1,
            arena: "n",
            key: 2,
            entries: vec![],
            staged_inserts: vec![2],
        }]
    );

    let err = old_store()
        .edit()
        .tap_flush(|tx| {
            tx.insert_marks(NId(99), MId(0));
        })
        .expect_err("oob key");
    assert_eq!(
        err.violations,
        vec![Violation::UnknownOldId { op: 0, arena: "n", id: 99 }]
    );
}

/// A per-module flush tap: stage, then flush, in one expression (the
/// tx_flush_shapes pattern, instantiated against this module's `Tx`).
trait TapFlush {
    fn tap_flush(
        self,
        f: impl FnOnce(&mut Tx<'_>),
    ) -> Result<FlushOutcome, FlushError>;
}

impl TapFlush for Tx<'_> {
    fn tap_flush(
        mut self,
        f: impl FnOnce(&mut Tx<'_>),
    ) -> Result<FlushOutcome, FlushError> {
        f(&mut self);
        self.flush()
    }
}
