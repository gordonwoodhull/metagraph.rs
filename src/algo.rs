//! Traversal algorithms written against the atomic relations, not against any
//! backend. Anything with out-edges whose edges have targets can be walked.
//!
//! The bounds carry one marker parameter per arrow trait (`OutEdges<MO>`,
//! `Target<MT>`) — inferred at call sites while each handle carries at most
//! one impl of each trait; fused handles in M2 qualify with partial
//! turbofish (`bfs::<_, _, markers::outer::OutEdges, _>(h)`).

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

use crate::stdlib::{OutEdges, Target};

/// Breadth-first visit order from `start`, each node yielded once.
pub fn bfs<N, E, MO, MT>(start: N) -> Vec<N>
where
    N: Copy + Eq + Hash + OutEdges<MO, Edge = E>,
    E: Target<MT, Node = N>,
{
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    let mut queue = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);
    while let Some(n) = queue.pop_front() {
        order.push(n);
        for e in n.out_edges() {
            let m = e.target();
            if visited.insert(m) {
                queue.push_back(m);
            }
        }
    }
    order
}

/// Depth-first preorder from `start`, each node yielded once.
pub fn dfs<N, E, MO, MT>(start: N) -> Vec<N>
where
    N: Copy + Eq + Hash + OutEdges<MO, Edge = E>,
    E: Target<MT, Node = N>,
{
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    let mut stack = vec![start];
    while let Some(n) = stack.pop() {
        if !visited.insert(n) {
            continue;
        }
        order.push(n);
        for e in n.out_edges() {
            stack.push(e.target());
        }
    }
    order
}
