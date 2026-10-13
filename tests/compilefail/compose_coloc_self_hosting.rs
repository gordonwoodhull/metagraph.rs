// compose-time docking check, self-hosting arm: an instance's root
// fused with its own element — the instance's arenas would embed into
// their own records. The legal host shape is cross-instance (one
// instance's element hosting another instance's root). The arm was
// pinned at the M3 round-4 merge; this fixture rode with its
// implementation.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        SelfHost {
            instances: d: metagraph::stdlib::Digraph;
            d.Graph <=> d.Node;
        }
    }
}

fn main() {}
