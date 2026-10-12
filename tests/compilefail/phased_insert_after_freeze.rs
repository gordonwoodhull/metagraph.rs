// The type-state store's compile-fail proof 1: insert-after-freeze. The
// frozen phase carries no write ops — `freeze()` consumed the build store,
// and `insert_out` exists only on `Store<Build>`.
use metagraph_exper::implement;

implement! {
    m2_fixtures::shapes::CsrDigraph as Store {
        nodes: arena,
        out:   csr_adjacency,
    }
}

fn main() {
    let mut s = Store::new();
    let x = s.add_n();
    let y = s.add_n();
    let e = s.add_e(x, y);
    s.insert_out(x, e);
    let f = s.freeze();
    f.insert_out(x, e); // E0599: no method `insert_out` on `Store<Frozen>`
}
