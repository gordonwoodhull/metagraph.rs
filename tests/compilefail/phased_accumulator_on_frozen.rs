// The type-state store's compile-fail proof 3: no accumulators on the
// frozen type. The phase attribution is real at the type level — the
// `on store build` field's Frozen type is `()` through the store's phase
// trait, so the pre-compaction buckets are unwritable (and unreadable as
// anything but unit) after `freeze()`.
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
    let mut f = s.freeze();
    f.buckets.push(Vec::new()); // E0599: no method `push` on `()`
}
