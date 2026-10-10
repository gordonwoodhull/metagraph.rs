//! Atomic graph traversal relations — one trait per arrow.
//!
//! Each relation is its own trait: the implementor (`Self`) is on the left of
//! the arrow, the associated type on the right. No trait bounds relate the
//! traits to each other; those constraints belong on algorithms, so that the
//! relations stay freely composable.
//!
//! Iterators are returned through GATs so each backend can name its own
//! borrowing iterator type without boxing.

use std::iter::Chain;

/// G -> N : the nodes of a graph
pub trait Nodes {
    type Node;
    type Nodes<'a>: Iterator<Item = Self::Node>
    where
        Self: 'a;
    fn nodes(&self) -> Self::Nodes<'_>;
}

/// N -> E : edges entering a node
pub trait InEdges {
    type Edge;
    type InEdges<'a>: Iterator<Item = Self::Edge>
    where
        Self: 'a;
    fn in_edges(&self) -> Self::InEdges<'_>;
}

/// N -> E : edges leaving a node
pub trait OutEdges {
    type Edge;
    type OutEdges<'a>: Iterator<Item = Self::Edge>
    where
        Self: 'a;
    fn out_edges(&self) -> Self::OutEdges<'_>;
}

/// N -> E : all edges incident on a node, out-edges first, then in-edges.
///
/// A self-loop appears twice, once per direction.
pub trait Edges {
    type Edge;
    type Edges<'a>: Iterator<Item = Self::Edge>
    where
        Self: 'a;
    fn edges(&self) -> Self::Edges<'_>;
}

/// Derived, never implemented by hand: any type with out- and in-edges of the
/// same edge type gets `Edges` by chaining the two. The cost of this
/// convenience is that no backend can override it with a hand-optimized
/// implementation (no specialization on stable Rust).
impl<T> Edges for T
where
    T: OutEdges + InEdges<Edge = <T as OutEdges>::Edge>,
{
    type Edge = <T as OutEdges>::Edge;
    type Edges<'a>
        = Chain<<T as OutEdges>::OutEdges<'a>, <T as InEdges>::InEdges<'a>>
    where
        Self: 'a;

    fn edges(&self) -> Self::Edges<'_> {
        self.out_edges().chain(self.in_edges())
    }
}

/// E -> N : the node an edge points from
pub trait Source {
    type Node;
    fn source(&self) -> Self::Node;
}

/// E -> N : the node an edge points to
pub trait Target {
    type Node;
    fn target(&self) -> Self::Node;
}
