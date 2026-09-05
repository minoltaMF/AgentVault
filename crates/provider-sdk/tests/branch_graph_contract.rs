use provider_sdk::{BranchGraph, BranchGraphError, BranchNode};

fn ids(nodes: Vec<&BranchNode>) -> Vec<&str> {
    nodes.into_iter().map(|node| node.id.as_str()).collect()
}

#[test]
fn graph_preserves_sibling_branches_and_resolves_the_active_path() {
    let graph = BranchGraph::try_new(
        vec![
            BranchNode::new("root", None, 0),
            BranchNode::new("left", Some("root".into()), 1),
            BranchNode::new("right", Some("root".into()), 2),
            BranchNode::new("right-leaf", Some("right".into()), 3),
        ],
        Some("right-leaf".into()),
    )
    .expect("valid branch graph");

    assert_eq!(
        graph
            .nodes()
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "left", "right", "right-leaf"]
    );
    assert_eq!(ids(graph.roots()), vec!["root"]);
    assert_eq!(ids(graph.leaves()), vec!["left", "right-leaf"]);
    assert_eq!(
        ids(graph.active_path().expect("active path")),
        vec!["root", "right", "right-leaf"]
    );
}

#[test]
fn graph_rejects_ambiguous_or_broken_native_lineage() {
    let duplicate = BranchGraph::try_new(
        vec![
            BranchNode::new("same", None, 0),
            BranchNode::new("same", None, 1),
        ],
        Some("same".into()),
    )
    .expect_err("duplicate native ids must be rejected");
    assert!(matches!(
        duplicate,
        BranchGraphError::DuplicateNodeId { ref id } if id == "same"
    ));

    let orphan = BranchGraph::try_new(
        vec![BranchNode::new("child", Some("missing".into()), 0)],
        Some("child".into()),
    )
    .expect_err("missing parents must be rejected");
    assert!(matches!(
        orphan,
        BranchGraphError::MissingParent { ref node_id, ref parent_id }
            if node_id == "child" && parent_id == "missing"
    ));

    let cycle = BranchGraph::try_new(
        vec![
            BranchNode::new("a", Some("b".into()), 0),
            BranchNode::new("b", Some("a".into()), 1),
        ],
        Some("b".into()),
    )
    .expect_err("cycles must be rejected");
    assert!(matches!(cycle, BranchGraphError::Cycle { .. }));
}
