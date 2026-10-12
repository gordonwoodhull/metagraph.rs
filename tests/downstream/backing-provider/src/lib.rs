//! A downstream crate that *declares* a user-facing backing — the
//! cross-crate arm of M3 step 2's bounce gate. The declaration's
//! `on_insert` body names a free helper (`note_push`) that exists only at
//! the backing's use site (spike 5, caveat 2: item paths resolve at the
//! use site, not the declaration site), so this crate compiles on its own
//! while the body stays unresolvable until a use site provides the helper.

pub mod lists {
    use metagraph_exper::backing;

    // An intrusive list that reports every push through a use-site helper —
    // structurally the stdlib's `intrusive_list` (push-front, reverse
    // insertion order, exclusive membership) plus the report.
    backing! {
        counting_list: A -> B, many {
            on A { first: Option<BId> = stem }
            on B { next: Option<BId> = stem }
            iter Walk { cur: Option<BId> }
            on_insert(a, b) {
                note_push(a, b);
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
}
