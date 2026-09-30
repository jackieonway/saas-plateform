use std::collections::HashMap;
use uuid::Uuid;

/// 通用 JSON 树构建: 输入平铺数组 (每项含 "id" 与 "parentId" 可空),
/// 按 parentId 组织为嵌套树 (每项附 "children"), 父节点不在集合内的项视为根。
pub fn build_tree(items: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut nodes: HashMap<Uuid, serde_json::Map<String, serde_json::Value>> = HashMap::new();
    let mut parent_of: HashMap<Uuid, Option<Uuid>> = HashMap::new();
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
            parent_of.insert(id, pid);
            nodes.insert(id, v.as_object().cloned().unwrap_or_default());
        }
    }
    // 孩子索引 (父节点不存在于集合中 → 视为根; 自引用防护)
    let mut children: HashMap<Option<Uuid>, Vec<Uuid>> = HashMap::new();
    for (id, pid) in &parent_of {
        let effective = pid.filter(|p| nodes.contains_key(p) && *p != *id);
        children.entry(effective).or_default().push(*id);
    }

    fn render(
        id: Uuid,
        nodes: &HashMap<Uuid, serde_json::Map<String, serde_json::Value>>,
        children: &HashMap<Option<Uuid>, Vec<Uuid>>,
        depth: usize,
    ) -> serde_json::Value {
        let mut node = nodes.get(&id).cloned().unwrap_or_default();
        let kids = children
            .get(&Some(id))
            .map(|v| {
                v.iter()
                    .map(|c| render(*c, nodes, children, depth + 1))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        node.insert("children".into(), serde_json::Value::Array(kids));
        serde_json::Value::Object(node)
    }

    children
        .get(&None)
        .map(|roots| roots.iter().map(|r| render(*r, &nodes, &children, 0)).collect())
        .unwrap_or_default()
}
