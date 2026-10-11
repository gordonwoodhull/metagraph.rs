//! A downstream crate declaring its *own* root single-arrow patterns and
//! implementing a pattern that instantiates them. Roots register themselves
//! through the same courier mechanism as the stdlib's — the stdlib enjoys
//! no privilege — so user-declared roots are implementable exactly like
//! stdlib ones, addressed by `crate::`-rooted schema paths.
//!
//! Note: patterns live in a named module (`shapes`), like the stdlib's own
//! declarations. A pattern declared at the crate root would collide with
//! its own courier's `#[macro_export]` crate-root anchor (E0255).

mod shapes {
    use metagraph_exper::pattern;

    // The user's roots: fresh traits, owned and registered here. Read-side
    // identity: `.tags()` yielding `Tag` handles through the GAT `Tags`,
    // and `.leaves()` yielding `Leaf` handles through the GAT `Leaves`.
    pattern! { Tags   { entities: root Post, Tag;  arrows { tags:   Post -> Tag, many; } } }
    pattern! { Leaves { entities: root Tag, Leaf; arrows { leaves: Tag -> Leaf, many; } } }

    pattern! {
        Labeled {
            entities: root Post, Tag, Leaf;
            arrows {
                tags:   crate::shapes::Tags,   Post -> Tag;
                leaves: crate::shapes::Leaves, Tag -> Leaf;
            }
        }
    }
}

use metagraph_exper::implement;

implement! {
    crate::shapes::Labeled as LabeledStore {
        tags:   arena,
        leaves: intrusive_list,
    }
}

#[cfg(test)]
mod tests {
    use crate::{LeafId, LabeledStore, TagId};
    use crate::shapes::{Leaves, Tags};

    #[test]
    fn own_root_instantiation_works() {
        let mut s = LabeledStore::new();
        // `Post` is the root-marked entity: no constructor, no count — the
        // store *is* the post. Build everything before taking handles
        // (handles freeze the store).
        let t0 = s.add_tag();
        let t1 = s.add_tag();
        let l0 = s.add_leaf();
        let l1 = s.add_leaf();
        let l2 = s.add_leaf();
        s.insert_leaves(t0, l0);
        s.insert_leaves(t0, l1);
        s.insert_leaves(t1, l2);
        assert_eq!(s.tag_count(), 2);
        assert_eq!(s.leaf_count(), 3);
        assert!(s.validate().is_ok());

        // Root-sourced `arena` arrow: a record is a member by existing.
        let post = s.post();
        let tags: Vec<TagId> = post.tags().map(|t| t.id()).collect();
        assert_eq!(tags, [TagId(0), TagId(1)]);

        // Element-sourced primitive `intrusive_list` arrow: explicit
        // insert op, reverse insertion order (push-front).
        let leaves: Vec<LeafId> = post
            .tags()
            .find(|t| t.id() == t0)
            .unwrap()
            .leaves()
            .map(|l| l.id())
            .collect();
        assert_eq!(leaves, [LeafId(1), LeafId(0)]);
        let t1_leaves: Vec<LeafId> = post
            .tags()
            .find(|t| t.id() == t1)
            .unwrap()
            .leaves()
            .map(|l| l.id())
            .collect();
        assert_eq!(t1_leaves, [LeafId(2)]);

        // The traits are the user's own roots': method, GAT, and associated
        // type names come from the roots' declarations — resolvable by
        // path from a dependent position.
        fn assert_nameable() {
            let _ = core::any::type_name::<<crate::Post<'_> as Tags<crate::shapes::labeled::markers::Tags>>::Tag>();
        }
        assert_nameable();
    }
}
