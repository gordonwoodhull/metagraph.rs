// Predicate evaluation (M3 step 7): two arrows targeting one class both
// bound to a `membership: existence` backing — B's entity arrow is unique,
// so the duplicate is rejected naming both arrows.
use metagraph_exper::implement;

implement! {
    m2_fixtures::neg::TwoInto as TwoStore {
        m1: arena,
        m2: arena,
    }
}

fn main() {}
