//! The bounce gate (M3 step 2): one `implement!` invocation resolving two
//! *distinct* backing couriers — a stdlib backing by bare ident (resolved
//! against the runtime crate) and a user backing by path (resolved through
//! a renamed dependency) — with hook bodies spliced hygienically. The
//! user backing's `on_insert` names `note_push`, defined here at the use
//! site and nowhere in the declaring crate (spike 5, caveat 2); the
//! declaration cannot even mention its signature, so the count it keeps is
//! the proof the call resolved *here*.

use std::sync::atomic::{AtomicU32, Ordering};

static PUSHES: AtomicU32 = AtomicU32::new(0);

fn note_push<A, B>(_a: A, _b: B) {
    PUSHES.fetch_add(1, Ordering::SeqCst);
}

mod shapes {
    use metagraph::pattern;

    pattern! {
        Tagged {
            entities: root Doc, Tag, Link;
            arrows {
                tags:  metagraph::stdlib::Nodes,    Doc -> Tag;
                links: metagraph::stdlib::OutEdges, Tag -> Link;
            }
        }
    }
}

use metagraph::implement;

implement! {
    crate::shapes::Tagged as TaggedStore {
        tags:  arena,
        links: prov::lists::counting_list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metagraph::stdlib::{Nodes, OutEdges};

    #[test]
    fn two_couriers_one_invocation() {
        let mut s = TaggedStore::new();
        let t0 = s.add_tag();
        let t1 = s.add_tag();
        let l0 = s.add_link();
        let l1 = s.add_link();
        let l2 = s.add_link();
        s.insert_links(t0, l1);
        s.insert_links(t0, l0);
        s.insert_links(t1, l2);

        // The use-site helper ran once per push through the user backing's
        // spliced hook body — resolution and hygiene together.
        assert_eq!(PUSHES.load(Ordering::SeqCst), 3);
        assert_eq!(s.tag_count(), 2);
        assert_eq!(s.link_count(), 3);
        assert!(s.validate().is_ok());

        // Arena iteration is forward; the user list iterates in reverse
        // insertion order (push-front is authored in the declaration).
        // Trait identity is the root's (`nodes`/`out_edges`), never the
        // instantiation's local alias.
        let doc = s.doc();
        let tags: Vec<u32> = doc.nodes().map(|t| t.id().0).collect();
        assert_eq!(tags, [0, 1]);
        let t0 = doc.nodes().next().unwrap();
        let links: Vec<u32> = t0.out_edges().map(|l| l.id().0).collect();
        assert_eq!(links, [0, 1]);
        let t1 = doc.nodes().nth(1).unwrap();
        let links1: Vec<u32> = t1.out_edges().map(|l| l.id().0).collect();
        assert_eq!(links1, [2]);
    }
}
