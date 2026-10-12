// The type-state store's compile-fail proof 2: frozen-as-builder. The
// consuming `freeze()` moves the build store — there is nothing left to
// construct with, so the build phase cannot be reused after the boundary.
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
    let _e = s.add_e(x, x);
    let f = s.freeze();
    s.add_n(); // E0382: use of moved value `s`
    let _ = f;
}
