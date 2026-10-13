// M4 step 12/13: a phased store's transaction is borrowed-only
// (`into_edit`'s in-place path against live build-phase accumulators is
// not emitted — see the step-12 notes); the frozen store offers `edit()`
// but no `into_edit`.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as MixedStore {
        nodes:     arena,
        out_edges: csr_adjacency,
        in_edges:  intrusive_list,
    }
}

fn main() {
    let s = MixedStore::new();
    let f = s.freeze();
    let _tx = f.into_edit(); // E0599: no method named `into_edit`
}
