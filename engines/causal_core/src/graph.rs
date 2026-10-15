//! 因果图数据结构 + 基础操作 + do-calculus 干预
//!
//! 实现依据：19_因果一致性验证引擎.md §2.1
//!
//! 核心数据结构：
//! - [`CausalGraph`]：变量节点 + 因果边（带权重，正=促进，负=抑制）+ 出/入邻接表
//! - [`CausalNode`]：变量节点（id + name + kind）
//! - [`CausalEdge`]：因果边（from, to, weight）
//!
//! 关键算法：
//! - [`CausalGraph::intervene`]：do-calculus 干预，计算 do(X=x) 对所有后代的效应
//!   Effect(X→Y) = Σ_{path: X⇝Y} ∏_{(u→v)∈path} w_{u→v}
//!   使用 Kahn 拓扑排序 + 传播，O(V+E)，DAG 上数学确定

use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Errors returned when constructing a causal graph.
#[derive(Debug, Clone, PartialEq)]
pub enum CausalGraphError {
    NodeOutOfBounds { node: usize, node_count: usize },
    NonFiniteWeight,
    CycleDetected { from: usize, to: usize },
}

impl std::fmt::Display for CausalGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeOutOfBounds { node, node_count } => {
                write!(f, "node index {} is out of bounds for {} nodes", node, node_count)
            }
            Self::NonFiniteWeight => write!(f, "causal edge weight must be finite"),
            Self::CycleDetected { from, to } => {
                write!(f, "adding edge {} -> {} would introduce a cycle", from, to)
            }
        }
    }
}

impl std::error::Error for CausalGraphError {}

/// 变量类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VarKind {
    /// 连续变量（温度、价格、剂量）
    Continuous,
    /// 离散变量（开关、等级）
    Discrete,
    /// 类别变量（颜色、类别）
    Categorical,
}

/// 因果变量节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalNode {
    pub id: usize,
    pub name: String,
    pub kind: VarKind,
}

/// 因果边 — 正权重=促进，负权重=抑制
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CausalEdge {
    pub from: usize,
    pub to: usize,
    pub weight: f64,
}

/// 因果图 — 系统单一真理源（可动态扩展）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalGraph {
    nodes: Vec<CausalNode>,
    edges: Vec<CausalEdge>,
    name_to_id: HashMap<String, usize>,
    /// 出邻接表：u → [(v, w), ...]
    out_adj: HashMap<usize, Vec<(usize, f64)>>,
    /// 入邻接表：v ← [(u, w), ...]
    in_adj: HashMap<usize, Vec<(usize, f64)>>,
}

impl CausalGraph {
    /// 创建空因果图
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            name_to_id: HashMap::new(),
            out_adj: HashMap::new(),
            in_adj: HashMap::new(),
        }
    }

    /// 添加变量节点（幂等：同名重复调用返回已有 id）
    pub fn add_node(&mut self, name: &str) -> usize {
        self.add_node_with_kind(name, VarKind::Continuous)
    }

    /// 添加变量节点并指定类型
    pub fn add_node_with_kind(&mut self, name: &str, kind: VarKind) -> usize {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(CausalNode {
            id,
            name: name.to_string(),
            kind,
        });
        self.name_to_id.insert(name.to_string(), id);
        self.out_adj.insert(id, Vec::new());
        self.in_adj.insert(id, Vec::new());
        id
    }

    /// Add an edge only if it preserves the directed-acyclic-graph invariant.
    pub fn try_add_edge(
        &mut self,
        from: usize,
        to: usize,
        weight: f64,
    ) -> Result<(), CausalGraphError> {
        let node_count = self.nodes.len();
        if from >= node_count {
            return Err(CausalGraphError::NodeOutOfBounds { node: from, node_count });
        }
        if to >= node_count {
            return Err(CausalGraphError::NodeOutOfBounds { node: to, node_count });
        }
        if !weight.is_finite() {
            return Err(CausalGraphError::NonFiniteWeight);
        }
        if self.has_path(to, from) {
            return Err(CausalGraphError::CycleDetected { from, to });
        }

        self.edges.push(CausalEdge { from, to, weight });
        self.out_adj.entry(from).or_default().push((to, weight));
        self.in_adj.entry(to).or_default().push((from, weight));
        Ok(())
    }

    /// Compatibility API. Invalid edges fail fast in debug and release builds.
    /// Prefer try_add_edge when input may be invalid.
    pub fn add_edge(&mut self, from: usize, to: usize, weight: f64) {
        self.try_add_edge(from, to, weight)
            .expect("invalid edge for causal DAG");
    }

    fn has_path(&self, start: usize, target: usize) -> bool {
        let mut stack = vec![start];
        let mut visited = HashSet::new();
        while let Some(node) = stack.pop() {
            if node == target {
                return true;
            }
            if !visited.insert(node) {
                continue;
            }
            if let Some(neighbors) = self.out_adj.get(&node) {
                stack.extend(neighbors.iter().map(|(next, _)| *next));
            }
        }
        false
    }

    /// 按名称查找节点 id
    pub fn find_node(&self, name: &str) -> Option<usize> {
        self.name_to_id.get(name).copied()
    }

    /// 按 id 查询节点名称
    pub fn node_name(&self, id: usize) -> Option<&str> {
        self.nodes.get(id).map(|n| n.name.as_str())
    }

    /// 节点总数
    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// 边总数
    pub fn num_edges(&self) -> usize {
        self.edges.len()
    }

    /// 所有边（只读切片）
    pub fn all_edges(&self) -> &[CausalEdge] {
        &self.edges
    }

    /// 所有节点（只读切片）
    pub fn all_nodes(&self) -> &[CausalNode] {
        &self.nodes
    }

    /// 出邻居（u 直接指向的节点 + 权重）
    pub fn out_neighbors(&self, u: usize) -> Option<&[(usize, f64)]> {
        self.out_adj.get(&u).map(|v| v.as_slice())
    }

    /// 入邻居（直接指向 u 的节点 + 权重）
    pub fn in_neighbors(&self, u: usize) -> Option<&[(usize, f64)]> {
        self.in_adj.get(&u).map(|v| v.as_slice())
    }

    /// do-calculus 干预：计算 do(X=x_val) 对所有后代节点的总效应
    ///
    /// 数学定义：Effect(X→Y) = Σ_{path: X⇝Y} ∏_{(u→v)∈path} w_{u→v}
    ///
    /// 算法：Kahn 拓扑排序（限定 X 的可达后代子图）+ 按拓扑序传播效应
    /// 复杂度：O(V + E)
    ///
    /// 返回：Vec<(node_id, effect_value)>，effect_value = Effect(X→Y) * x_val
    ///       不包含 X 自身，只包含 |effect| > 1e-12 的后代
    pub fn intervene(&self, x: usize, x_val: f64) -> Vec<(usize, f64)> {
        let n = self.nodes.len();
        if n == 0 || x >= n {
            return Vec::new();
        }

        // effect[i] = X 对节点 i 的总效应（已乘以 x_val）
        let mut effect = vec![0.0f64; n];
        effect[x] = x_val;

        // Step 1: BFS 找出 X 的所有后代（可达节点）
        let mut reachable = vec![false; n];
        reachable[x] = true;
        let mut q: VecDeque<usize> = VecDeque::new();
        q.push_back(x);
        while let Some(u) = q.pop_front() {
            if let Some(outs) = self.out_adj.get(&u) {
                for &(v, _) in outs {
                    if !reachable[v] {
                        reachable[v] = true;
                        q.push_back(v);
                    }
                }
            }
        }

        // Step 2: 在可达子图上计算入度
        let mut in_degree = vec![0usize; n];
        for u in 0..n {
            if reachable[u] {
                if let Some(outs) = self.out_adj.get(&u) {
                    for &(v, _) in outs {
                        if reachable[v] {
                            in_degree[v] += 1;
                        }
                    }
                }
            }
        }

        // Step 3: Kahn 拓扑排序
        let mut kahn_q: VecDeque<usize> = (0..n)
            .filter(|&i| reachable[i] && in_degree[i] == 0)
            .collect();
        let mut order = Vec::new();
        while let Some(u) = kahn_q.pop_front() {
            order.push(u);
            if let Some(outs) = self.out_adj.get(&u) {
                for &(v, _) in outs {
                    if reachable[v] {
                        in_degree[v] -= 1;
                        if in_degree[v] == 0 {
                            kahn_q.push_back(v);
                        }
                    }
                }
            }
        }

        // 若 order 不包含所有可达节点，说明存在环；本实现按 DAG 假设，
        // 环上节点的 effect 不被传播（部分结果），调用方需保证 DAG 性
        let reachable_count = reachable.iter().filter(|&&r| r).count();
        let _ = reachable_count; // 仅用于调试，不强制 panic

        // Step 4: 按拓扑序传播效应
        for &u in &order {
            if let Some(outs) = self.out_adj.get(&u) {
                for &(v, w) in outs {
                    if reachable[v] {
                        effect[v] += effect[u] * w;
                    }
                }
            }
        }

        // Step 5: 收集非零后代（排除 X 自身）
        let mut result = Vec::new();
        for (i, &e) in effect.iter().enumerate() {
            if i != x && e.abs() > 1e-12 {
                result.push((i, e));
            }
        }
        result
    }

    /// 祖先集（包含自身）：用于后门准则、d-分离的 moralization
    pub(crate) fn ancestors_inclusive(&self, seeds: &[usize]) -> HashSet<usize> {
        let mut set: HashSet<usize> = seeds.iter().copied().collect();
        let mut stack: Vec<usize> = seeds.to_vec();
        while let Some(u) = stack.pop() {
            if let Some(parents) = self.in_adj.get(&u) {
                for &(p, _) in parents {
                    if set.insert(p) {
                        stack.push(p);
                    }
                }
            }
        }
        set
    }
}

impl Default for CausalGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────── 测试 ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_add_edge_rejects_invalid_indices_and_non_finite_weights() {
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");

        assert!(matches!(
            g.try_add_edge(a, 9, 1.0),
            Err(CausalGraphError::NodeOutOfBounds { node: 9, node_count: 2 })
        ));
        assert!(matches!(
            g.try_add_edge(a, b, f64::INFINITY),
            Err(CausalGraphError::NonFiniteWeight)
        ));
        assert_eq!(g.num_edges(), 0);
    }

    #[test]
    fn test_try_add_edge_rejects_cycles() {
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        g.add_edge(a, b, 1.0);
        g.add_edge(b, c, 1.0);

        assert!(matches!(
            g.try_add_edge(c, a, 1.0),
            Err(CausalGraphError::CycleDetected { from, to }) if from == c && to == a
        ));
        assert!(matches!(
            g.try_add_edge(a, a, 1.0),
            Err(CausalGraphError::CycleDetected { from, to }) if from == a && to == a
        ));
        assert_eq!(g.num_edges(), 2);
    }

    #[test]
    fn test_add_find_node_idempotent() {
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let a2 = g.add_node("A"); // 幂等
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(g.num_nodes(), 2);
        assert_eq!(g.find_node("A"), Some(a));
        assert_eq!(g.find_node("B"), Some(b));
        assert_eq!(g.find_node("missing"), None);
        assert_eq!(g.node_name(a), Some("A"));
    }

    #[test]
    fn test_intervene_chain() {
        // A → B → C, w=2.0 each; do(A=1.0) → B=2.0, C=4.0
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        g.add_edge(a, b, 2.0);
        g.add_edge(b, c, 2.0);

        let effects = g.intervene(a, 1.0);
        let map: HashMap<usize, f64> = effects.into_iter().collect();
        assert_eq!(map.len(), 2);
        assert!((map[&b] - 2.0).abs() < 1e-9);
        assert!((map[&c] - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_intervene_fork() {
        // A → B (0.5), A → C (-0.3); do(A=1) → B=0.5, C=-0.3
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        g.add_edge(a, b, 0.5);
        g.add_edge(a, c, -0.3);

        let effects = g.intervene(a, 1.0);
        let map: HashMap<usize, f64> = effects.into_iter().collect();
        assert!((map[&b] - 0.5).abs() < 1e-9);
        assert!((map[&c] - (-0.3)).abs() < 1e-9);
    }

    #[test]
    fn test_intervene_diamond() {
        // A → B → D, A → C → D; w=1.0 each
        // do(A=1) → B=1, C=1, D=2 (两条路径累加)
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        let d = g.add_node("D");
        g.add_edge(a, b, 1.0);
        g.add_edge(b, d, 1.0);
        g.add_edge(a, c, 1.0);
        g.add_edge(c, d, 1.0);

        let effects = g.intervene(a, 1.0);
        let map: HashMap<usize, f64> = effects.into_iter().collect();
        assert!((map[&b] - 1.0).abs() < 1e-9);
        assert!((map[&c] - 1.0).abs() < 1e-9);
        assert!((map[&d] - 2.0).abs() < 1e-9, "D should be 2.0 (two paths), got {}", map[&d]);
    }

    #[test]
    fn test_intervene_negative_weight_chain() {
        // A → B (-0.5) → C (-0.4); do(A=1) → B=-0.5, C=0.2 (负×负=正)
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        g.add_edge(a, b, -0.5);
        g.add_edge(b, c, -0.4);

        let effects = g.intervene(a, 1.0);
        let map: HashMap<usize, f64> = effects.into_iter().collect();
        assert!((map[&b] - (-0.5)).abs() < 1e-9);
        assert!((map[&c] - 0.2).abs() < 1e-9);
    }

    #[test]
    fn test_intervene_no_descendants() {
        // A 孤立节点；do(A=1) → 空
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let _b = g.add_node("B");
        let effects = g.intervene(a, 1.0);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_intervene_scale_with_x_val() {
        // A → B (0.5); do(A=2.0) → B=1.0
        let mut g = CausalGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        g.add_edge(a, b, 0.5);
        let effects = g.intervene(a, 2.0);
        let map: HashMap<usize, f64> = effects.into_iter().collect();
        assert!((map[&b] - 1.0).abs() < 1e-9);
    }
}
