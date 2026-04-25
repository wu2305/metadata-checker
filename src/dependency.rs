use crate::superpage::{ComponentExpr, RefType, SuperPageMetadata};
use std::collections::{HashMap, HashSet, VecDeque};

/// 组件间依赖关系
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// 组件 ID -> 它依赖的组件/变量列表
    pub dependencies: HashMap<String, Vec<RefType>>,
    /// 组件 ID -> 依赖于它的组件列表（反向依赖）
    pub reverse_deps: HashMap<String, Vec<String>>,
    /// 表达式信息
    pub expressions: HashMap<String, Vec<ComponentExpr>>,
}

impl DependencyGraph {
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
            for ref_type in &expr.refs {
                entry.push(ref_type.clone());
            }
            
            // 记录表达式
            exprs.entry(comp_id.clone())
                .or_insert_with(Vec::new)
                .push(expr.clone());

            // 建立反向依赖
            for ref_type in &expr.refs {
                match ref_type {
                    RefType::ComponentValue(ref_id) |
                    RefType::ComponentProperty(ref_id, _) => {
                        reverse.entry(ref_id.clone())
                            .or_insert_with(Vec::new)
                            .push(comp_id.clone());
                    }
                    _ => {}
                }
            }
        }

        DependencyGraph {
            dependencies: deps,
            reverse_deps: reverse,
            expressions: exprs,
        }
    }

    /// 获取拓扑排序（计算先后顺序）
    pub fn topological_sort(&self) -> Vec<String> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        // 初始化入度
        for (comp_id, refs) in &self.dependencies {
            in_degree.entry(comp_id.clone()).or_insert(0);
            for ref_type in refs {
                if let RefType::ComponentValue(dep_id) | RefType::ComponentProperty(dep_id, _) = ref_type {
                    if self.dependencies.contains_key(dep_id) {
                        *in_degree.entry(comp_id.clone()).or_insert(0) += 1;
                        adj.entry(dep_id.clone())
                            .or_insert_with(Vec::new)
                            .push(comp_id.clone());
                    }
                }
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        let mut result = Vec::new();

        // 找到所有入度为0的节点
        for (id, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(id.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            result.push(current.clone());
            
            if let Some(neighbors) = adj.get(&current) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(neighbor.clone());
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
                self.dfs_cycle(
                node,
                &mut visited,
                &mut rec_stack,
                &mut path,
                &mut cycles,
            );
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
                if let RefType::ComponentValue(dep_id) | RefType::ComponentProperty(dep_id, _) = ref_type {
                    if !visited.contains(dep_id) {
                        self.dfs_cycle(dep_id, visited, rec_stack, path, cycles);
                    } else if rec_stack.contains(dep_id) {
                        // 发现循环
                        if let Some(pos) = path.iter().position(|x| x == dep_id) {
                            let cycle: Vec<String> = path[pos..].iter().cloned().collect();
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
fn expand_expression(
    meta: &SuperPageMetadata,
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

    let clean = expr.trim_start_matches('=').trim_start_matches("${").trim_end_matches('}');
    let mut expanded = clean.to_string();

    // 获取当前组件的引用
    let refs = if let Some(exprs) = graph.expressions.get(component_id) {
        exprs.iter()
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
                            meta,
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
                        let pattern = format!("{}\\.value", regex_escape(dep_id));
                        let re = regex::Regex::new(&pattern).unwrap_or_else(|_| regex::Regex::new("NEVER_MATCH").unwrap());
                        expanded = re.replace_all(&expanded, &*dep_expanded).to_string();
                    }
                }
            }
            RefType::ModelField(model_id, field) => {
                let replacement = format!("({}.{})", model_id, field);
                let pattern = format!("{}\\.{}", regex_escape(model_id), regex_escape(field));
                let re = regex::Regex::new(&pattern).unwrap_or_else(|_| regex::Regex::new("NEVER_MATCH").unwrap());
                expanded = re.replace_all(&expanded, &*replacement).to_string();
            }
            RefType::Param(param_id) => {
                let replacement = format!("({})", param_id);
                let re = regex::Regex::new(&format!("\\b{}\\b", regex_escape(param_id)))
                    .unwrap_or_else(|_| regex::Regex::new("NEVER_MATCH").unwrap());
                expanded = re.replace_all(&expanded, &*replacement).to_string();
            }
            _ => {}
        }
    }

    format!("={}", expanded)
}

fn regex_escape(s: &str) -> String {
    s.replace("\\", "\\\\")
        .replace(".", "\\.")
        .replace("*", "\\*")
        .replace("+", "\\+")
        .replace("?", "\\?")
        .replace("[", "\\[")
        .replace("]", "\\]")
        .replace("(", "\\(")
        .replace(")", "\\)")
        .replace("{", "\\{")
        .replace("}", "\\}")
        .replace("^", "\\^")
        .replace("$", "\\$")
        .replace("|", "\\|")
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
