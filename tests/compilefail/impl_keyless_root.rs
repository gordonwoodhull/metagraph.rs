// implement!-time keyed check: the member pattern instantiates a keyed
// root but never binds a key sort — `key(LocalEntity)` is required at
// the instantiation site.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        K {
            instances: kk: m2_fixtures::neg::Keyless;
        }
    }
}

use metagraph::implement;

implement! {
    comp::K as KStore {
        kk.k: hash_index,
    }
}

fn main() {}
