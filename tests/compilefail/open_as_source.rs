// pattern!-time structural check: an open entity may only be the TARGET
// of arrows in its declaring pattern — it has no records of its own until
// a composition closes it, so nothing could ever source from it.
use metagraph::pattern;

pattern! {
    Bad {
        entities: root G, N, M(open);
        arrows { a: metagraph::stdlib::OutEdges, M -> N; }
    }
}

fn main() {}
