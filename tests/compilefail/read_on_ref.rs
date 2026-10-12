// M4 step 1: a provisional ref is an opaque token, not a handle — it has
// no read traits. Calling a read-side method on it is the type-system
// refusal the design leans on (naming and docs carry the rest).
use metagraph_exper::implement;
use metagraph_exper::stdlib::OutEdges;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn main() {
    let mut s = GraphStore::new();
    let mut tx = s.edit();
    let p = tx.add_node();
    let _ = p.out_edges(); // E0599: `Ref<NodeId>` has no method `out_edges`
}
