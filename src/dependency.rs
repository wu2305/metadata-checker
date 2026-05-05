use crate::superpage::{ComponentExpr, RefType, SuperPageMetadata};
use std::collections::{HashMap, HashSet};

/// 组件依赖关系分析模块
///
/// 基于 SuperPage 的表达式引用构建有向依赖图：
/// - 节点：组件 ID
/// - 边：组件 A 的表达式引用了组件 B 的值/属性
///
/// 提供拓扑排序（计算先后顺序）和循环检测功能。
/// 组件间依赖关系
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// 组件 ID -> 它依赖的组件/变量列表
    pub dependencies: HashMap<String, Vec<RefType>>,
    /// 组件 ID -> 依赖于它的组件列表（反向依赖）
    pub reverse_deps: HashMap<String, Vec<String>>,
    /// 表达式信息
    pub expressions: HashMap<String, Vec<ComponentExpr>>,
    /// 组件在元数据中的出现顺序（用于稳定拓扑排序）
    pub component_order: Vec<String>,
}

impl DependencyGraph {
    /// 从 SuperPage 元数据构建依赖图
    pub fn new(meta: &SuperPageMetadata) -> Self {
        let mut deps = HashMap::new();
        let mut reverse = HashMap::new();
        let mut exprs = HashMap::new();

        // 初始化所有组件
        for comp in &meta.components {
            deps.insert(comp.id.clone(), Vec::new());
            reverse.insert(comp.id.clone(), Vec::new());
        }

        // 构建依赖关系
        for expr in &meta.expressions {
            let comp_id = expr.component_id.clone();

            // 收集该组件的所有引用
            let entry = deps.entry(comp_id.clone()).or_insert_with(Vec::new);
            // validExp 等校验表达式中的自引用不应构成循环依赖
            let is_validation_field = expr.field == "validExp" || expr.field == "visibleCondition";
            for ref_type in &expr.refs {
                let is_self_ref = match ref_type {
                    RefType::ComponentValue(ref_id) | RefType::ComponentProperty(ref_id, _) => {
                        ref_id == &comp_id
                    }
                    _ => false,
                };
                // 仅在校验/条件字段中跳过自引用
                if is_validation_field && is_self_ref {
                    continue;
                }
                entry.push(ref_type.clone());
            }

            // 记录表达式
            exprs
                .entry(comp_id.clone())
                .or_insert_with(Vec::new)
                .push(expr.clone());

            // 建立反向依赖
            for ref_type in &expr.refs {
                match ref_type {
                    RefType::ComponentValue(ref_id) | RefType::ComponentProperty(ref_id, _) => {
                        let is_self_ref = ref_id == &comp_id;
                        if is_validation_field && is_self_ref {
                            continue;
                        }
                        reverse
                            .entry(ref_id.clone())
                            .or_insert_with(Vec::new)
                            .push(comp_id.clone());
                    }
                    _ => {}
                }
            }
        }

        let component_order: Vec<String> = meta.components.iter().map(|c| c.id.clone()).collect();

        DependencyGraph {
            dependencies: deps,
            reverse_deps: reverse,
            expressions: exprs,
            component_order,
        }
    }

    /// 获取拓扑排序（计算先后顺序）
    /// 拓扑排序（计算先后顺序）
    ///
    /// 稳定排序规则：
    /// - 入度为0的节点按元数据出现顺序处理
    /// - 当多个节点同时入度为0时，先出现的组件先输出
    /// - 这保证同一次解析多次运行结果一致
    pub fn topological_sort(&self) -> Vec<String> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        // 初始化入度
        for (comp_id, refs) in &self.dependencies {
            in_degree.entry(comp_id.clone()).or_insert(0);
            if refs.is_empty() {
                continue;
            }
            let comp_id_owned = comp_id.clone();
            for ref_type in refs {
                if let RefType::ComponentValue(dep_id) | RefType::ComponentProperty(dep_id, _) =
                    ref_type
                    && self.dependencies.contains_key(dep_id)
                {
                    *in_degree.entry(comp_id_owned.clone()).or_insert(0) += 1;
                    adj.entry(dep_id.clone())
                        .or_default()
                        .push(comp_id_owned.clone());
                }
            }
        }

        // 使用 Vec 作为队列，每次按 component_order 排序后取第一个
        let mut queue: Vec<String> = Vec::new();
        let mut result = Vec::new();

        // 找到所有入度为0的节点
        for (id, degree) in &in_degree {
            if *degree == 0 {
                queue.push(id.clone());
            }
        }

        // 按元数据出现顺序稳定排序入度为0的节点
        queue.sort_by_key(|id| {
            self.component_order
                .iter()
                .position(|o| o == id)
                .unwrap_or(usize::MAX)
        });

        while !queue.is_empty() {
            let current = queue.remove(0);
            result.push(current.clone());

            if let Some(neighbors) = adj.get(&current) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(neighbor.clone());
                        }
                    }
                }
            }

            // 重新排序，保证稳定性
            queue.sort_by_key(|id| {
                self.component_order
                    .iter()
                    .position(|o| o == id)
                    .unwrap_or(usize::MAX)
            });
        }

        result
    }

    /// 检测循环依赖
    pub fn detect_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for node in self.dependencies.keys() {
            if !visited.contains(node) {
                self.dfs_cycle(node, &mut visited, &mut rec_stack, &mut path, &mut cycles);
            }
        }

        cycles
    }

    fn dfs_cycle(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(refs) = self.dependencies.get(node) {
            for ref_type in refs {
                if let RefType::ComponentValue(dep_id) | RefType::ComponentProperty(dep_id, _) =
                    ref_type
                {
                    if !visited.contains(dep_id) {
                        self.dfs_cycle(dep_id, visited, rec_stack, path, cycles);
                    } else if rec_stack.contains(dep_id) {
                        // 发现循环
                        if let Some(pos) = path.iter().position(|x| x == dep_id) {
                            let cycle: Vec<String> = path[pos..].to_vec();
                            cycles.push(cycle);
                        }
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }
}

/// 值来源追溯结果
#[derive(Debug, Clone)]
pub struct ValueTrace {
    pub component_id: String,
    pub field: String,
    pub raw_expr: String,
    pub expanded_expr: String,
    pub source_chain: Vec<SourceNode>,
    pub is_external_input: bool,
    pub source_type: SourceType,
}

/// 值的来源类型
#[derive(Debug, Clone)]
pub enum SourceType {
    /// 页面参数输入
    Param,
    /// 用户输入
    UserInput,
    /// 数据模型自动获取
    ModelAuto,
    /// 系统变量
    System,
    /// 组件间计算
    Computed,
    /// 常量
    Constant,
    /// 未知
    Unknown,
}

#[derive(Debug, Clone)]
pub struct SourceNode {
    pub component_id: String,
    pub expr: String,
    pub refs: Vec<RefType>,
    pub source_type: SourceType,
}

/// 追溯组件值的来源
pub fn trace_value_source(
    meta: &SuperPageMetadata,
    graph: &DependencyGraph,
    target_component_id: &str,
    field: &str,
    max_depth: usize,
) -> Option<ValueTrace> {
    let expressions = graph.expressions.get(target_component_id)?;
    let expr = expressions.iter().find(|e| e.field == field)?;

    let mut source_chain = Vec::new();
    let mut visited = HashSet::new();

    let expanded = expand_expression(
        meta,
        graph,
        target_component_id,
        &expr.raw_expr,
        &mut source_chain,
        &mut visited,
        max_depth,
    );

    // 判断最终来源类型
    let source_type = determine_source_type(&expr.raw_expr, &source_chain);

    let is_external_input = matches!(
        source_type,
        SourceType::Param | SourceType::UserInput | SourceType::System
    );

    Some(ValueTrace {
        component_id: target_component_id.to_string(),
        field: field.to_string(),
        raw_expr: expr.raw_expr.clone(),
        expanded_expr: expanded,
        source_chain,
        is_external_input,
        source_type,
    })
}

/// 展开表达式，递归替换引用
pub fn expand_expression(
    _meta: &SuperPageMetadata,
    graph: &DependencyGraph,
    component_id: &str,
    expr: &str,
    source_chain: &mut Vec<SourceNode>,
    visited: &mut HashSet<String>,
    depth: usize,
) -> String {
    if depth == 0 {
        return expr.to_string();
    }

    let key = format!("{}.{}", component_id, expr);
    if visited.contains(&key) {
        return expr.to_string(); // 防止循环
    }
    visited.insert(key);

    let clean = expr
        .trim_start_matches('=')
        .trim_start_matches("${")
        .trim_end_matches('}');
    let mut expanded = clean.to_string();

    // 获取当前组件的引用
    let refs = if let Some(exprs) = graph.expressions.get(component_id) {
        exprs
            .iter()
            .find(|e| e.raw_expr == expr)
            .map(|e| e.refs.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    for ref_type in &refs {
        match ref_type {
            RefType::ComponentValue(dep_id) => {
                if let Some(dep_exprs) = graph.expressions.get(dep_id) {
                    // 优先查找 value 字段
                    if let Some(dep_expr) = dep_exprs.iter().find(|e| e.field == "value") {
                        let dep_expanded = expand_expression(
                            _meta,
                            graph,
                            dep_id,
                            &dep_expr.raw_expr,
                            source_chain,
                            visited,
                            depth - 1,
                        );

                        source_chain.push(SourceNode {
                            component_id: dep_id.clone(),
                            expr: dep_expr.raw_expr.clone(),
                            refs: dep_expr.refs.clone(),
                            source_type: determine_source_type(&dep_expr.raw_expr, &[]),
                        });

                        // 替换引用
                        expanded = replace_with_boundary(
                            &expanded,
                            &format!("{}.value", dep_id),
                            &dep_expanded,
                        );
                    }
                }
            }
            RefType::ModelField(model_id, field) => {
                let replacement = format!("({}.{})", model_id, field);
                expanded = replace_with_boundary(
                    &expanded,
                    &format!("{}.{}", model_id, field),
                    &replacement,
                );
            }
            RefType::Param(param_id) => {
                let replacement = format!("({})", param_id);
                expanded = replace_with_boundary(&expanded, param_id, &replacement);
            }
            _ => {}
        }
    }

    format!("={}", expanded)
}
/// Replace pattern only at word boundaries (word chars = [A-Za-z0-9_]).
fn replace_with_boundary(s: &str, pattern: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(s.len() + replacement.len());
    let pat_bytes = pattern.as_bytes();
    let s_bytes = s.as_bytes();
    let mut i = 0;

    while i <= s_bytes.len().saturating_sub(pat_bytes.len()) {
        if &s_bytes[i..i + pat_bytes.len()] == pat_bytes {
            let prev_ok = i == 0 || !is_word_char(s_bytes[i - 1]);
            let next_ok =
                i + pat_bytes.len() == s_bytes.len() || !is_word_char(s_bytes[i + pat_bytes.len()]);
            if prev_ok && next_ok {
                result.push_str(replacement);
                i += pat_bytes.len();
                continue;
            }
        }
        result.push(s_bytes[i] as char);
        i += 1;
    }
    while i < s_bytes.len() {
        result.push(s_bytes[i] as char);
        i += 1;
    }
    result
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 确定来源类型
fn determine_source_type(expr: &str, _chain: &[SourceNode]) -> SourceType {
    if expr.starts_with("=$user") || expr.contains("$user") {
        return SourceType::System;
    }
    if expr.starts_with("=$project") || expr.contains("$project") {
        return SourceType::System;
    }
    if expr.starts_with("=param") || expr.contains("param") && !expr.contains("=") {
        return SourceType::Param;
    }
    if expr.starts_with("=model") || expr.contains("model") {
        return SourceType::ModelAuto;
    }
    if !expr.starts_with('=') && !expr.starts_with("${") {
        return SourceType::Constant;
    }
    if expr.contains(".value") || expr.contains(".checked") {
        return SourceType::Computed;
    }
    SourceType::Unknown
}
