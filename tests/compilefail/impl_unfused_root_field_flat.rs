// implement!-time orchestrator rule, the flat arm: `r` is a `one` arrow
// sourced at the pattern's root, so `field` (the default backing, which
// carries `ctor_arg`) has no self record to live on — the root owns no
// record (it *is* the store). The composed arm is pinned by
// impl_unfused_root_field.rs.
use metagraph::implement;

implement! {
    m2_fixtures::neg::RootSourced as RootStore {}
}

fn main() {}
