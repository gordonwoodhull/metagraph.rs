// M4 step 13: a generic transform bounded on a write trait at the WRONG
// entity marker finds no impl at instantiation — the marker-parameterized
// write traits (Add/Remove/HandleToRef, one marker per entity) refuse the
// mismatch at compile time instead of silently wiring the wrong equality
// web. `Tx` implements `AddNode<markers::Node>` and `HandleToRef` at the
// per-entity markers only; the Edge entity's marker and the arrow's read
// marker name no write impl.
use metagraph_exper::implement;
use metagraph_exper::stdlib::digraph::{markers, AddNode};
use metagraph_exper::tx::HandleToRef;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn node_verb_at_edge_marker<T: AddNode<markers::Edge>>() {}

fn handle_map_at_arrow_marker<T: HandleToRef<markers::OutEdges>>() {}

fn main() {
    let s = GraphStore::new();
    let tx = s.edit();
    node_verb_at_edge_marker::<Tx<'_>>();
    handle_map_at_arrow_marker::<Tx<'_>>();
}
