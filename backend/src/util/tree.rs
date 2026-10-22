use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// 通用 JSON 树构建: 输入平铺数组 (每项含 "id" 与 "parentId" 可空),
/// 按 parentId 组织为嵌套树 (每项附 "children"), 父节点不在集合内的项视为根。
/// 保留输入中的兄弟节点顺序；历史循环数据按首次出现的节点断开，避免整组节点丢失。
pub fn build_tree(items: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut nodes: HashMap<Uuid, serde_json::Map<String, serde_json::Value>> = HashMap::new();
    let mut parent_of: HashMap<Uuid, Option<Uuid>> = HashMap::new();
    let mut ordered_ids = Vec::new();
    for v in items {
        let id = v
            .get("id")
            .and_then(|x| x.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        let pid = v
            .get("parentId")
            .and_then(|x| x.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        if let Some(id) = id {
            if !nodes.contains_key(&id) {
                ordered_ids.push(id);
            }
            parent_of.insert(id, pid);
            nodes.insert(id, v.as_object().cloned().unwrap_or_default());
        }
    }
    // 孩子索引 (父节点不存在于集合中 → 视为根; 自引用防护)
    let mut children: HashMap<Option<Uuid>, Vec<Uuid>> = HashMap::new();
    for id in &ordered_ids {
        let effective = parent_of[id].filter(|p| nodes.contains_key(p) && *p != *id);
        children.entry(effective).or_default().push(*id);
    }

    fn render(
        id: Uuid,
        nodes: &HashMap<Uuid, serde_json::Map<String, serde_json::Value>>,
        children: &HashMap<Option<Uuid>, Vec<Uuid>>,
        emitted: &mut HashSet<Uuid>,
    ) -> serde_json::Value {
        emitted.insert(id);
        let mut node = nodes.get(&id).cloned().unwrap_or_default();
        let mut kids = Vec::new();
        if let Some(child_ids) = children.get(&Some(id)) {
            for child_id in child_ids {
                if !emitted.contains(child_id) {
                    kids.push(render(*child_id, nodes, children, emitted));
                }
            }
        }
        node.insert("children".into(), serde_json::Value::Array(kids));
        serde_json::Value::Object(node)
    }

    let mut emitted = HashSet::new();
    let mut roots = Vec::new();
    if let Some(root_ids) = children.get(&None) {
        for id in root_ids {
            roots.push(render(*id, &nodes, &children, &mut emitted));
        }
    }
    // A cycle has no natural root. Emit one deterministic root per remaining
    // component, and let the visited set cut its back edge.
    for id in ordered_ids {
        if !emitted.contains(&id) {
            roots.push(render(id, &nodes, &children, &mut emitted));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn node(id: u128, parent: Option<u128>) -> Value {
        json!({"id": Uuid::from_u128(id), "parentId": parent.map(Uuid::from_u128)})
    }

    #[test]
    fn preserves_input_order_for_roots_and_children() {
        let items = vec![
            node(30, None),
            node(13, Some(30)),
            node(20, None),
            node(12, Some(30)),
            node(11, Some(30)),
        ];
        for _ in 0..32 {
            let tree = build_tree(items.clone());
            assert_eq!(tree[0]["id"], json!(Uuid::from_u128(30)));
            assert_eq!(tree[1]["id"], json!(Uuid::from_u128(20)));
            let children = tree[0]["children"].as_array().unwrap();
            let ids: Vec<_> = children.iter().map(|node| node["id"].clone()).collect();
            assert_eq!(ids, [13, 12, 11].map(|id| json!(Uuid::from_u128(id))));
        }
    }

    #[test]
    fn roots_orphans_and_self_references_without_duplicates() {
        let tree = build_tree(vec![node(1, Some(99)), node(2, Some(2)), node(3, None)]);
        assert_eq!(tree.len(), 3);
        assert!(tree.iter().all(|node| node["children"] == json!([])));
        assert!(build_tree(vec![]).is_empty());
    }

    #[test]
    fn retains_each_node_of_a_cycle_exactly_once() {
        let tree = build_tree(vec![
            node(1, Some(2)),
            node(2, Some(1)),
            node(3, Some(2)),
            node(4, None),
        ]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0]["id"], json!(Uuid::from_u128(4)));
        assert_eq!(tree[1]["id"], json!(Uuid::from_u128(1)));
        assert_eq!(tree[1]["children"][0]["id"], json!(Uuid::from_u128(2)));
        assert_eq!(
            tree[1]["children"][0]["children"],
            json!([{"id": Uuid::from_u128(3), "parentId": Uuid::from_u128(2), "children": []}])
        );
    }
}
