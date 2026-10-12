// implement!-time home check: a `field`-backed arrow sourced at an
// unfused root — the root owns no record (it *is* the store).
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        RootComp {
            instances: rs: m2_fixtures::neg::RootSourced;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::RootComp as RootStore {
        // `r` is a `one` arrow, so `field` is the default backing.
    }
}

fn main() {}
