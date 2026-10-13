// compose-time structural check (classes_legal): peer-shaped
// cross-instance colocation — elements of two different instances have
// no root to anchor the merged arena. The host shape (one element
// hosting another instance's root) is the legal cross-instance form.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph;
            outer.Node <=> inner.Node;
        }
    }
}

fn main() {}
