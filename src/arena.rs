//! Arena-backed graph storage with handle types that play the role of wide
//! pointers: each handle is a `&GraphStore` plus an index, `Copy` and 16
//! bytes, presenting an object-oriented graph over flat `Vec` arenas.
//!
//! Adjacency is stored as intrusive linked lists threaded through the edge
//! records: each node holds the head of its out-list and in-list, each edge
//! holds the next links. Appending a node or edge is O(1). Lists are in
//! reverse insertion order.
//!
//! Handles borrow the store, so the store cannot be mutated while handles are
//! live and indices cannot dangle.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::traits::{InEdges, Nodes, OutEdges, Source, Target};

/// Index of a node in a [`GraphStore`] arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(pub u32);

/// Index of an edge in a [`GraphStore`] arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EdgeId(pub u32);

#[derive(Default, Debug)]
pub struct GraphStore {
    nodes: Vec<NodeData>,
    edges: Vec<EdgeData>,
}

#[derive(Debug)]
struct NodeData {
    first_out: Option<EdgeId>,
    first_in: Option<EdgeId>,
}

#[derive(Debug)]
struct EdgeData {
    source: NodeId,
    target: NodeId,
    next_out: Option<EdgeId>, // next in source's out-list
    next_in: Option<EdgeId>,  // next in target's in-list
}

impl GraphStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(NodeData {
            first_out: None,
            first_in: None,
        });
        id
    }

    pub fn add_edge(&mut self, source: NodeId, target: NodeId) -> EdgeId {
        let id = EdgeId(self.edges.len() as u32);
        let next_out = self.nodes[source.0 as usize].first_out;
        let next_in = self.nodes[target.0 as usize].first_in;
        self.edges.push(EdgeData {
            source,
            target,
            next_out,
            next_in,
        });
        self.nodes[source.0 as usize].first_out = Some(id);
        self.nodes[target.0 as usize].first_in = Some(id);
        id
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// The root handle: the "object" view of this store.
    pub fn graph(&self) -> Graph<'_> {
        Graph { store: self }
    }
}

/// Handle on a whole graph: just a store reference, no index.
#[derive(Clone, Copy)]
pub struct Graph<'g> {
    store: &'g GraphStore,
}

/// Handle on a node: store reference plus node index.
#[derive(Clone, Copy)]
pub struct Node<'g> {
    store: &'g GraphStore,
    id: NodeId,
}

/// Handle on an edge: store reference plus edge index.
#[derive(Clone, Copy)]
pub struct Edge<'g> {
    store: &'g GraphStore,
    id: EdgeId,
}

impl<'g> Graph<'g> {
    pub fn new(store: &'g GraphStore) -> Self {
        Self { store }
    }
}

impl<'g> Node<'g> {
    pub fn id(self) -> NodeId {
        self.id
    }
}

impl<'g> Edge<'g> {
    pub fn id(self) -> EdgeId {
        self.id
    }
}

// Handles are equal only if they point into the same store at the same index.
// Deriving would deep-compare the stores through the references.

impl PartialEq for Node<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.store, other.store)
    }
}
impl Eq for Node<'_> {}
impl Hash for Node<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.store as *const GraphStore).hash(state);
        self.id.hash(state);
    }
}
impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({:?})", self.id)
    }
}

impl PartialEq for Edge<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.store, other.store)
    }
}
impl Eq for Edge<'_> {}
impl Hash for Edge<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.store as *const GraphStore).hash(state);
        self.id.hash(state);
    }
}
impl fmt::Debug for Edge<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Edge({:?})", self.id)
    }
}

impl fmt::Debug for Graph<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("node_count", &self.store.node_count())
            .field("edge_count", &self.store.edge_count())
            .finish()
    }
}

pub struct NodesIter<'g> {
    store: &'g GraphStore,
    next: u32,
    len: u32,
}

impl<'g> Iterator for NodesIter<'g> {
    type Item = Node<'g>;

    fn next(&mut self) -> Option<Node<'g>> {
        (self.next < self.len).then(|| {
            let n = Node {
                store: self.store,
                id: NodeId(self.next),
            };
            self.next += 1;
            n
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.len - self.next) as usize;
        (remaining, Some(remaining))
    }
}

/// Walks one adjacency list — out-list or in-list, selected by which head and
/// link field the node hands it.
pub struct EdgeIter<'g> {
    store: &'g GraphStore,
    next: Option<EdgeId>,
    follow: fn(&EdgeData) -> Option<EdgeId>,
}

impl<'g> Iterator for EdgeIter<'g> {
    type Item = Edge<'g>;

    fn next(&mut self) -> Option<Edge<'g>> {
        let id = self.next?;
        let data = &self.store.edges[id.0 as usize];
        self.next = (self.follow)(data);
        Some(Edge {
            store: self.store,
            id,
        })
    }
}

impl<'g> Nodes for Graph<'g> {
    type Node = Node<'g>;
    type Nodes<'a>
        = NodesIter<'g>
    where
        Self: 'a;

    fn nodes(&self) -> NodesIter<'g> {
        NodesIter {
            store: self.store,
            next: 0,
            len: self.store.nodes.len() as u32,
        }
    }
}

impl<'g> OutEdges for Node<'g> {
    type Edge = Edge<'g>;
    type OutEdges<'a>
        = EdgeIter<'g>
    where
        Self: 'a;

    fn out_edges(&self) -> EdgeIter<'g> {
        EdgeIter {
            store: self.store,
            next: self.store.nodes[self.id.0 as usize].first_out,
            follow: |e| e.next_out,
        }
    }
}

impl<'g> InEdges for Node<'g> {
    type Edge = Edge<'g>;
    type InEdges<'a>
        = EdgeIter<'g>
    where
        Self: 'a;

    fn in_edges(&self) -> EdgeIter<'g> {
        EdgeIter {
            store: self.store,
            next: self.store.nodes[self.id.0 as usize].first_in,
            follow: |e| e.next_in,
        }
    }
}

// `Edges` comes from the blanket impl in `traits`: out-edges chained with
// in-edges.

impl<'g> Source for Edge<'g> {
    type Node = Node<'g>;

    fn source(&self) -> Node<'g> {
        Node {
            store: self.store,
            id: self.store.edges[self.id.0 as usize].source,
        }
    }
}

impl<'g> Target for Edge<'g> {
    type Node = Node<'g>;

    fn target(&self) -> Node<'g> {
        Node {
            store: self.store,
            id: self.store.edges[self.id.0 as usize].target,
        }
    }
}
