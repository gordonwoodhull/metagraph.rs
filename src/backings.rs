//! The standard library backings: the four built-ins of M1/M2 (`field`,
//! `arena`, `intrusive_list`, `hash_index`) re-implemented as ordinary
//! `backing!` declarations, plus `csr_adjacency` (the phased second
//! `many` backing). No fixed table survives this: `implement!` resolves
//! every one of them through its backing courier, exactly as it resolves
//! root arrows through registration couriers — the bare binding spelling
//! (`arena`) names the declaration in this module; a user backing is
//! addressed by the path of the module holding its declaration.
//!
//! Bodies are ordinary Rust over the closed role vocabulary (`a`, `b`,
//! `k`, `rec(x)`, `records(E)`, `maps(A)`, `host_of(v)`, `b_arena_of`/
//! `k_arena_of`, `map_from`, `store`, `AId`/`BId`/`KId`, `idx(id)`, the
//! reserved `determining(x)` and `handle(x)`); codegen binds the roles per
//! placement. Contributed fields are referenced by their *logical* names
//! (`value`, `first`, `next`, `index`) — the naming formula clauses state
//! how each renames per binding (`= arrow`, `= stem`, `= suffix`).
//!
//! Codegen additionally binds three scaffold names in every validate and
//! hook body: `errs` (the `Vec<String>` sink), `arrow_label` (a `&str`
//! like `` "arrow `sub.subnode`" ``), and `det_label` (the determinant
//! arrow's name, `validate by` only). A uniform range pre-pass over
//! contributed `Option<id>` fields runs before each authored validate
//! body, so walks may assume links in range and use `rec(x)` freely.

use metagraph_exper_macros::backing;

// M1's `source`/`target`: one typed field on the self record, fed by the
// constructor argument named after the arrow, read directly.
backing! {
    field: A -> B, one {
        on A { value: BId = arrow }
        ctor_arg
        read(a) { rec(a).value }
        validate {
            for ra in records(A) {
                let v = rec(ra).value;
                if idx(v) >= b_arena_of(host_of(v)).len() {
                    errs.push(format!(
                        "{}: {:?} points at {:?}, which is not a live record",
                        arrow_label, ra, v
                    ));
                }
            }
        }
    }
}

// M1's `nodes`: existence membership — the write path is `add_<B>` itself;
// iteration is an index range over the target's arena.
backing! {
    arena: A -> B, many {
        membership: existence
        iter Walk { cur: u32, len: u32 }
        iterate(a) { Walk { cur: 0, len: b_arena_of(a).len() as u32 } }
        advance(w) {
            (w.cur < w.len).then(|| {
                let n = handle(w.cur);
                w.cur += 1;
                n
            })
        }
        requires: hosts_arena
        // validate: nothing — existence cannot dangle
    }
}

// M1's `out_edges`/`in_edges`: a singly-linked list per self object,
// pushed at the front (reverse insertion order), exclusive within one
// arrow's lists.
backing! {
    intrusive_list: A -> B, many {
        on A { first: Option<BId> = stem }
        on B { next: Option<BId> = stem }
        iter Walk { cur: Option<BId> }
        on_insert(a, b) {
            rec(b).next = rec(a).first;
            rec(a).first = Some(b);
        }
        iterate(a) { Walk { cur: rec(a).first } }
        advance(w) {
            let id = w.cur?;
            w.cur = rec(id).next;
            Some(handle(id))
        }
        iteration_order: reverse_insertion
        contract: exclusive_membership
        requires: direct_records
        validate by {
            let mut seen_by_self: HashMap<AId, HashSet<BId>> = HashMap::new();
            for ra in records(A) {
                let mut walk: HashSet<BId> = HashSet::new();
                let mut cur = rec(ra).first;
                while let Some(mid) = cur {
                    if determining(mid) != ra {
                        errs.push(format!(
                            "{}: {:?} is in the list of {:?} but its `{}` field says {:?} — membership contradicts the determinant",
                            arrow_label, mid, ra, det_label, determining(mid)
                        ));
                    }
                    if !walk.insert(mid) {
                        errs.push(format!(
                            "{}: the list of {:?} repeats {:?} — a cycle; traversal would diverge",
                            arrow_label, ra, mid
                        ));
                        break;
                    }
                    seen_by_self.entry(ra).or_default().insert(mid);
                    cur = rec(mid).next;
                }
            }
            for rb in records(B) {
                let listed = seen_by_self
                    .get(&determining(rb))
                    .is_some_and(|members| members.contains(&rb));
                if !listed {
                    errs.push(format!(
                        "{}: {:?} is missing from the list of its `{}` ({:?})",
                        arrow_label, rb, det_label, determining(rb)
                    ));
                }
            }
        }
        validate primitive {
            let mut seen: HashSet<BId> = HashSet::new();
            for ra in records(A) {
                let mut walk: HashSet<BId> = HashSet::new();
                let mut cur = rec(ra).first;
                while let Some(mid) = cur {
                    if !walk.insert(mid) {
                        errs.push(format!(
                            "{}: the list of {:?} repeats {:?} — a cycle; traversal would diverge",
                            arrow_label, ra, mid
                        ));
                        break;
                    }
                    if !seen.insert(mid) {
                        errs.push(format!(
                            "{}: {:?} appears in more than one list — shared membership; the insert silently truncated the older list",
                            arrow_label, mid
                        ));
                        break;
                    }
                    cur = rec(mid).next;
                }
            }
        }
    }
}

// M2's `subnode`/`subedge`: a per-self-object `HashMap` from key id to
// target id (store-level when the self is an unfused root), inserted
// check-then-write — a keyed arrow is a function.
backing! {
    hash_index: A -> B, key(K) {
        on A { index: HashMap<KId, BId> = suffix }
        on_insert(a, k, b) {
            if map_from(a).contains_key(&k) {
                panic!("duplicate key {:?} for arrow `{}`: a keyed arrow is a function", k, arrow_label);
            }
            map_from(a).insert(k, b);
        }
        lookup(a, k) { map_from(a).get(&k).copied() }
        validate {
            for (owner, m) in maps(A) {
                for (k, v) in m.iter() {
                    if idx(k) >= k_arena_of(host_of(k)).len() {
                        errs.push(format!(
                            "{}: key {:?} does not name a record",
                            arrow_label, k
                        ));
                    }
                    if idx(v) >= b_arena_of(host_of(v)).len() {
                        errs.push(format!(
                            "{}: value {:?} of key {:?} on {:?} is not a live record",
                            arrow_label, v, k, owner
                        ));
                    }
                    if host_of(v) != owner {
                        errs.push(format!(
                            "{}: value {:?} of key {:?} is anchored at the wrong host (self is {:?})",
                            arrow_label, v, k, owner
                        ));
                    }
                }
            }
        }
    }
}

// The phased second `many` backing (spike 6): accumulate into per-self
// buckets during build, compact to offset ranges at the single `freeze()`,
// iterate as a forward slice walk. The phase-attributed contributions are
// the partition the type-state store keys on.
backing! {
    csr_adjacency: A -> B, many {
        on store build { buckets: Vec<Vec<BId>> }
        on store frozen { offsets: Vec<u32>, adj: Vec<BId> }
        on_add(a) { store.buckets.push(Vec::new()) }
        on_insert(a, b) { store.buckets[idx(a)].push(b) }
        compact {
            let mut offsets = Vec::with_capacity(store.buckets.len() + 1);
            let mut adj = Vec::new();
            offsets.push(0u32);
            for bucket in store.buckets.iter() {
                adj.extend(bucket.iter().copied());
                offsets.push(adj.len() as u32);
            }
            store.offsets = offsets;
            store.adj = adj;
            store.buckets = Vec::new();
        }
        iter Walk { lo: u32, hi: u32 }
        iterate(a) { Walk { lo: store.offsets[idx(a)], hi: store.offsets[idx(a) + 1] } }
        advance(w) {
            (w.lo < w.hi).then(|| {
                let id = store.adj[w.lo as usize];
                w.lo += 1;
                handle(id)
            })
        }
        iteration_order: forward_insertion
        phase: append_only_build, requires_compaction_at_boundary,
               freeze_scope: per_store, read_precondition: post_compaction
        requires: direct_records
        validate {
            if store.offsets.len() != records(A).count() + 1 {
                errs.push(format!(
                    "{}: offsets length {} does not match the arena length {}",
                    arrow_label, store.offsets.len(), records(A).count()
                ));
            }
            if store.offsets.first() != Some(&0) || store.offsets.last() != Some(&(store.adj.len() as u32)) {
                errs.push(format!(
                    "{}: offsets span [{:?}, {:?}] does not start at 0 and end at the adjacency length {}",
                    arrow_label, store.offsets.first(), store.offsets.last(), store.adj.len()
                ));
            }
            for w in store.offsets.windows(2) {
                if w[0] > w[1] {
                    errs.push(format!(
                        "{}: offsets are not monotonic at [{}, {}]",
                        arrow_label, w[0], w[1]
                    ));
                }
            }
        }
    }
}
