use crate::superpage::{ComponentExpr, ComponentValueForm, RefType, SuperPageMetadata};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

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
                    RefType::ComponentValue(ref_id, _) | RefType::ComponentProperty(ref_id, _) => {
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
                    RefType::ComponentValue(ref_id, _) | RefType::ComponentProperty(ref_id, _) => {
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
                if let RefType::ComponentValue(dep_id, _) | RefType::ComponentProperty(dep_id, _) =
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

        let component_order: HashMap<&str, usize> = self
            .component_order
            .iter()
            .enumerate()
            .map(|(index, id)| (id.as_str(), index))
            .collect();
        let mut queue: BinaryHeap<Reverse<(usize, String)>> = BinaryHeap::new();
        let mut result = Vec::new();

        // 找到所有入度为0的节点
        for (id, degree) in &in_degree {
            if *degree == 0 {
                queue.push(Reverse((
                    component_order
                        .get(id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX),
                    id.clone(),
                )));
            }
        }

        while let Some(Reverse((_, current))) = queue.pop() {
            result.push(current.clone());

            if let Some(neighbors) = adj.get(&current) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(Reverse((
                                component_order
                                    .get(neighbor.as_str())
                                    .copied()
                                    .unwrap_or(usize::MAX),
                                neighbor.clone(),
                            )));
                        }
                    }
                }
            }
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
                if let RefType::ComponentValue(dep_id, _) | RefType::ComponentProperty(dep_id, _) =
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
    /// 未完成原因；为空才表示本次支持范围内完整展开。
    pub issues: Vec<TraceIssue>,
    pub source_chain: Vec<SourceNode>,
    pub is_external_input: bool,
    pub source_type: SourceType,
}

/// 追溯未完成的结构化证据，输出层必须保留。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TraceIssue {
    pub code: &'static str,
    pub component_id: String,
    pub token: String,
}

/// 单次追溯状态；循环只看当前递归栈，不把共享上游当成环。
#[derive(Default)]
struct TraceState {
    stack: HashSet<(String, String)>,
    chain: Vec<SourceNode>,
    issues: Vec<TraceIssue>,
}

impl TraceState {
    /// 同一未完成原因只记录一次，保持首次遇到的顺序。
    fn issue(&mut self, code: &'static str, component_id: &str, token: &str) {
        let issue = TraceIssue {
            code,
            component_id: component_id.to_string(),
            token: token.to_string(),
        };
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }
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

    let mut state = TraceState::default();
    let expanded = expand_trace(
        meta,
        graph,
        target_component_id,
        field,
        &expr.raw_expr,
        &mut state,
        max_depth,
    );
    let source_chain = state.chain;
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
        issues: state.issues,
        source_chain,
        is_external_input,
        source_type,
    })
}

/// 按原串引用区间一次性渲染，插入片段永远不被再次扫描。
fn expand_trace(
    meta: &SuperPageMetadata,
    graph: &DependencyGraph,
    component_id: &str,
    field: &str,
    expression: &str,
    state: &mut TraceState,
    depth: usize,
) -> String {
    if depth == 0 {
        state.issue("TRACE_DEPTH_LIMIT", component_id, expression);
        return expression.to_string();
    }
    let key = (component_id.to_string(), field.to_string());
    if !state.stack.insert(key.clone()) {
        state.issue("TRACE_CYCLE", component_id, expression);
        return expression.to_string();
    }
    let clean = expression.strip_prefix('=').unwrap_or(expression);
    let parsed = crate::superpage::parse_expression_ast(clean);
    if !parsed.diagnostics.is_empty() {
        state.issue("TRACE_UNSUPPORTED_EXPRESSION", component_id, expression);
    }
    let component_ids = meta
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect();
    let source_ids = meta
        .sources
        .iter()
        .map(|source| source.id.as_str())
        .collect();
    let param_ids = meta.params.iter().map(|param| param.id.as_str()).collect();
    let mut result = String::new();
    let mut cursor = 0;
    for occurrence in crate::superpage::reference_occurrences(clean) {
        let (reference, _) = crate::superpage::resolve_ref_type(
            &occurrence.reference,
            &component_ids,
            &source_ids,
            &param_ids,
        );
        let value_target = match &reference {
            RefType::ComponentValue(id, ComponentValueForm::Value | ComponentValueForm::Bare) => {
                Some(id)
            }
            RefType::ComponentProperty(id, property) if property == "value" => Some(id),
            _ => None,
        };
        let replacement =
            if let Some(target) = value_target {
                if let Some(target_expr) = graph
                    .expressions
                    .get(target)
                    .and_then(|expressions| expressions.iter().find(|expr| expr.field == "value"))
                {
                    let expanded = expand_trace(
                        meta,
                        graph,
                        target,
                        "value",
                        &target_expr.raw_expr,
                        state,
                        depth - 1,
                    );
                    if !state.chain.iter().any(|node| {
                        node.component_id == *target && node.expr == target_expr.raw_expr
                    }) {
                        state.chain.push(SourceNode {
                            component_id: target.clone(),
                            expr: target_expr.raw_expr.clone(),
                            refs: target_expr.refs.clone(),
                            source_type: determine_source_type(&target_expr.raw_expr, &[]),
                        });
                    }
                    let fragment = expanded.strip_prefix('=').unwrap_or(&expanded);
                    let composite = matches!(
                        crate::superpage::parse_expression_ast(fragment).ast,
                        Some(
                            crate::superpage::AstNode::BinaryOp { .. }
                                | crate::superpage::AstNode::Conditional { .. }
                                | crate::superpage::AstNode::UnaryOp { .. }
                        )
                    );
                    if composite && occurrence.range != (0..clean.len()) {
                        Some(format!("({fragment})"))
                    } else {
                        Some(fragment.to_string())
                    }
                } else {
                    state.issue("TRACE_MISSING_VALUE", component_id, &occurrence.token);
                    None
                }
            } else {
                match reference {
                    RefType::ModelField(model, field) => Some(format!("({model}.{field})")),
                    RefType::Param(param) => Some(format!("({param})")),
                    RefType::Other(_) => {
                        state.issue(
                            "TRACE_UNRESOLVED_REFERENCE",
                            component_id,
                            &occurrence.token,
                        );
                        None
                    }
                    RefType::ComponentProperty(_, _)
                    | RefType::ComponentValue(_, ComponentValueForm::Suffix) => {
                        state.issue(
                            "TRACE_PROPERTY_NOT_EXPANDED",
                            component_id,
                            &occurrence.token,
                        );
                        None
                    }
                    _ => None,
                }
            };
        result.push_str(&clean[cursor..occurrence.range.start]);
        result.push_str(replacement.as_deref().unwrap_or(&occurrence.token));
        cursor = occurrence.range.end;
    }
    result.push_str(&clean[cursor..]);
    state.stack.remove(&key);
    format!("={result}")
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
