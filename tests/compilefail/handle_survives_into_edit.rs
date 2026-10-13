// M4 step 13: handle survival under `into_edit` — a handle borrows the
// store; consuming the store into an owned transaction while a handle is
// alive is the E0505 the pass discipline leans on (`edit()` is the
// escape hatch: the borrowed transaction coexists with live handles).
use metagraph_exper::implement;
use metagraph_exper::stdlib::Nodes;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn main() {
    let s = GraphStore::new();
    let h = s.graph().nodes().next().unwrap();
    let _tx = s.into_edit(); // E0505: cannot move out of `s` because it is borrowed
    drop(h);
}
