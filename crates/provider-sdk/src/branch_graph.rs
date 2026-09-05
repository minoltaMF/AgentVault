use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

/// Provider-native event identity and parentage in append order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub ordinal: u64,
}

impl BranchNode {
    pub fn new(id: impl Into<String>, parent_id: Option<String>, ordinal: u64) -> Self {
        Self {
            id: id.into(),
            parent_id,
            ordinal,
        }
    }
}

/// A validated provider-native branch graph.
///
/// Nodes remain in their original append order. The active node identifies the native session's
/// current leaf, while sibling branches remain available for a complete branch atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchGraph {
    nodes: Vec<BranchNode>,
    active_node_id: Option<String>,
}

impl BranchGraph {
    pub fn try_new(
        nodes: Vec<BranchNode>,
        active_node_id: Option<String>,
    ) -> Result<Self, BranchGraphError> {
        let mut indexes = HashMap::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            if node.id.trim().is_empty() {
                return Err(BranchGraphError::EmptyNodeId {
                    ordinal: node.ordinal,
                });
            }
            if indexes.insert(node.id.as_str(), index).is_some() {
                return Err(BranchGraphError::DuplicateNodeId {
                    id: node.id.clone(),
                });
            }
        }

        for node in &nodes {
            if let Some(parent_id) = node.parent_id.as_deref() {
                if !indexes.contains_key(parent_id) {
                    return Err(BranchGraphError::MissingParent {
                        node_id: node.id.clone(),
                        parent_id: parent_id.to_string(),
                    });
                }
            }
        }

        if let Some(active_node_id) = active_node_id.as_deref() {
            if !indexes.contains_key(active_node_id) {
                return Err(BranchGraphError::ActiveNodeMissing {
                    node_id: active_node_id.to_string(),
                });
            }
        }

        validate_acyclic(&nodes, &indexes)?;
        Ok(Self {
            nodes,
            active_node_id,
        })
    }

    pub fn nodes(&self) -> &[BranchNode] {
        &self.nodes
    }

    pub fn active_node_id(&self) -> Option<&str> {
        self.active_node_id.as_deref()
    }

    pub fn roots(&self) -> Vec<&BranchNode> {
        self.nodes
            .iter()
            .filter(|node| node.parent_id.is_none())
            .collect()
    }

    pub fn leaves(&self) -> Vec<&BranchNode> {
        let parents = self
            .nodes
            .iter()
            .filter_map(|node| node.parent_id.as_deref())
            .collect::<HashSet<_>>();
        self.nodes
            .iter()
            .filter(|node| !parents.contains(node.id.as_str()))
            .collect()
    }

    pub fn path_to(&self, node_id: &str) -> Result<Vec<&BranchNode>, BranchGraphError> {
        let indexes = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut current =
            indexes
                .get(node_id)
                .copied()
                .ok_or_else(|| BranchGraphError::UnknownNode {
                    node_id: node_id.to_string(),
                })?;
        let mut reversed = Vec::new();

        loop {
            let node = &self.nodes[current];
            reversed.push(node);
            let Some(parent_id) = node.parent_id.as_deref() else {
                break;
            };
            current = indexes
                .get(parent_id)
                .copied()
                .expect("branch parents were validated at construction");
        }

        reversed.reverse();
        Ok(reversed)
    }

    pub fn active_path(&self) -> Result<Vec<&BranchNode>, BranchGraphError> {
        match self.active_node_id() {
            Some(node_id) => self.path_to(node_id),
            None => Ok(Vec::new()),
        }
    }
}

fn validate_acyclic(
    nodes: &[BranchNode],
    indexes: &HashMap<&str, usize>,
) -> Result<(), BranchGraphError> {
    // 0 = unseen, 1 = in the current parent chain, 2 = fully validated.
    let mut state = vec![0_u8; nodes.len()];
    for start in 0..nodes.len() {
        if state[start] != 0 {
            continue;
        }
        let mut chain = Vec::new();
        let mut current = Some(start);
        while let Some(index) = current {
            match state[index] {
                0 => {
                    state[index] = 1;
                    chain.push(index);
                    current = nodes[index].parent_id.as_deref().map(|parent_id| {
                        indexes
                            .get(parent_id)
                            .copied()
                            .expect("branch parents were validated before cycle detection")
                    });
                }
                1 => {
                    return Err(BranchGraphError::Cycle {
                        node_id: nodes[index].id.clone(),
                    });
                }
                _ => break,
            }
        }
        for index in chain {
            state[index] = 2;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchGraphError {
    EmptyNodeId { ordinal: u64 },
    DuplicateNodeId { id: String },
    MissingParent { node_id: String, parent_id: String },
    ActiveNodeMissing { node_id: String },
    UnknownNode { node_id: String },
    Cycle { node_id: String },
}

impl fmt::Display for BranchGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNodeId { ordinal } => {
                write!(
                    formatter,
                    "branch node at ordinal {ordinal} has an empty id"
                )
            }
            Self::DuplicateNodeId { id } => write!(formatter, "duplicate branch node id: {id}"),
            Self::MissingParent { node_id, parent_id } => write!(
                formatter,
                "branch node {node_id} references missing parent {parent_id}"
            ),
            Self::ActiveNodeMissing { node_id } => {
                write!(formatter, "active branch node does not exist: {node_id}")
            }
            Self::UnknownNode { node_id } => write!(formatter, "unknown branch node: {node_id}"),
            Self::Cycle { node_id } => {
                write!(formatter, "branch graph contains a cycle at {node_id}")
            }
        }
    }
}

impl Error for BranchGraphError {}
