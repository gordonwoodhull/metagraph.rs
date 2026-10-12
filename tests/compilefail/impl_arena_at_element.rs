// Predicate evaluation (M3 step 7), the flat `hosts_arena` arm: `m2` is
// sourced at the element `M`, but a flat store hosts every arena at the
// store — B's arena is not hosted by the self record, so `arena` binds
// only root-sourced arrows. `m1` binds legally to show `m2` is the
// rejected one.
use metagraph_exper::implement;

implement! {
    m2_fixtures::neg::TwoInto as TwoStore {
        m1: intrusive_list,
        m2: arena,
    }
}

fn main() {}
