// implement!-time rejection, the flat arm: a phased backing bound to an
// arrow sourced at the pattern's root — the entity-arrow/unfused-root
// case. The root owns no record (it *is* the store), so there is no self
// id for `on_add`/`on_insert`, and a root-level B arena under an unfused
// root is the target's entity-arrow shape, which a hook-maintained
// backing cannot declare.
use metagraph_exper::implement;

implement! {
    m2_fixtures::neg::RootMany as Store {
        cs: csr_adjacency,
    }
}

fn main() {}
