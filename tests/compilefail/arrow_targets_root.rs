// pattern!-time structural check: no arrow may TARGET the root-marked
// entity — after the no-arena rule it owns no id type to store. Arrows
// sourced at it are fine.
use metagraph::pattern;

pattern! {
    Bad {
        entities: root A, B;
        arrows {
            a: stdlib::Nodes, A -> B;
            b: stdlib::OutEdges, B -> A;
        }
    }
}

fn main() {}
