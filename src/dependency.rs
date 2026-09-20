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
            RefType::ComponentValue(dep_id, form) => {
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

                        // 替换引用。被替换进去的是表达式**片段**，不带前导 `=`；
                        // 否则会拼出 `CONCAT(a, =(param1))` 这种嵌套等号。
                        let fragment = dep_expanded.strip_prefix('=').unwrap_or(&dep_expanded);
                        expanded = replace_component_value_ref(&expanded, dep_id, *form, &fragment);
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
/// 把表达式里对 `dep_id` 这个组件的**值引用**替换成它自己的展开式。
///
/// 替换 pattern 由引用的来源文法（`ComponentValueForm`，spec A1b）决定，
/// 不再按后缀猜测：
/// - `Value`：pattern 为 `dep_id.value`，显式取值；
/// - `Bare`：pattern 为裸 `dep_id`，且后面不能跟 `.`——裸引用后面若还跟着
///   `.`，取的是别的属性，不是值；
/// - `Suffix`（`dep_id.step` 等）：引用的是组件其它属性，替换成值的展开式
///   是错的，整段跳过。
///
/// 单遍扫描：替换文本不会被本函数再次扫描，避免展开式里恰好含 `dep_id`
/// 时被二次替换。字符串字面量整段跳过（见 `string_literal_spans`）。
fn replace_component_value_ref(
    s: &str,
    dep_id: &str,
    form: ComponentValueForm,
    replacement: &str,
) -> String {
    const VALUE_SUFFIX: &str = ".value";

    if dep_id.is_empty() {
        return s.to_string();
    }
    if form == ComponentValueForm::Suffix {
        return s.to_string();
    }

    let pattern = match form {
        ComponentValueForm::Value => format!("{}{}", dep_id, VALUE_SUFFIX),
        _ => dep_id.to_string(),
    };

    let bytes = s.as_bytes();
    let literals = string_literal_spans(s);
    let mut result = String::with_capacity(s.len() + replacement.len());
    let mut last = 0usize;

    for (start, matched) in s.match_indices(&pattern) {
        // 落在上一次替换吃掉的区间内。
        if start < last {
            continue;
        }
        // 字符串字面量里的同名文本不是引用。
        if in_string_literal(&literals, start) {
            continue;
        }
        // 词首边界：`bdep` 这类更长标识符的前缀不算引用。
        if start > 0 && is_word_char(bytes[start - 1]) {
            continue;
        }

        let end = start + matched.len();
        // 词尾边界：`dep_id.values` / `dep_idx` 之类的更长标识符不算引用；
        // 裸引用后面若还跟着 `.`，取的是别的属性，不是值。
        if bytes.get(end).is_some_and(|b| is_word_char(*b)) {
            continue;
        }
        if form == ComponentValueForm::Bare && bytes.get(end) == Some(&b'.') {
            continue;
        }

        result.push_str(&s[last..start]);
        result.push_str(replacement);
        last = end;
    }

    result.push_str(&s[last..]);
    result
}

/// 表达式里**字符串字面量**所覆盖的字节区间（含两侧引号）。
///
/// 词法与 `superpage::expr_ast::Tokenizer::read_string_literal` 同口径：单引号与
/// 双引号各自成对，反斜杠转义下一个字符，未闭合的引号一直吃到串尾。
///
/// 替换点靠它整段跳过字面量。`CONCAT("b", b.value)` 里的 `"b"` 是**文本**，
/// 不是对组件 `b` 的引用；把它换成 b 的展开式会**静默改变表达式的含义**
/// （`CONCAT("b", 1)` → `CONCAT("1", 1)`），且没有任何诊断。
/// 词边界判定挡不住这种情况——引号本身就不是词字符，边界检查照样通过。
fn string_literal_spans(s: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut chars = s.char_indices();
    while let Some((start, c)) = chars.next() {
        if c != '\'' && c != '"' {
            continue;
        }
        // 未闭合的引号：与 tokenizer 一致，吃到串尾。
        let mut end = s.len();
        while let Some((idx, cc)) = chars.next() {
            if cc == '\\' {
                // 转义：连同被转义的那个字符一起跳过，`"a\\"b"` 不算在此闭合。
                chars.next();
            } else if cc == c {
                end = idx + cc.len_utf8();
                break;
            }
        }
        spans.push((start, end));
    }
    spans
}

fn in_string_literal(spans: &[(usize, usize)], pos: usize) -> bool {
    spans.iter().any(|(start, end)| pos >= *start && pos < *end)
}

/// 只在词边界（词字符 = `[A-Za-z0-9_]`）处替换 pattern。
///
/// 按 `str` 的字符边界切片拼接，不做逐字节 `u8 as char` 转换——后者会把中文等
/// 多字节 UTF-8 拆成乱码。`match_indices` 只在合法字符边界上给出匹配，且
/// pattern 长于 s 时直接不产生匹配，因此不存在越界切片。
fn replace_with_boundary(s: &str, pattern: &str, replacement: &str) -> String {
    if pattern.is_empty() {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len() + replacement.len());
    let s_bytes = s.as_bytes();
    let literals = string_literal_spans(s);
    let mut last = 0usize;

    for (start, matched) in s.match_indices(pattern) {
        // 与 `replace_component_value_ref` 同一条理由：`CONCAT("param1", param1)`
        // 的前一个 `param1` 是文本。Param / ModelField 走的是本函数，坑一模一样。
        if in_string_literal(&literals, start) {
            continue;
        }
        let end = start + matched.len();
        // 边界字节若是多字节字符的一部分（>= 0x80），is_word_char 返回 false，
        // 即中文与 ASCII 词字符相邻时视为边界成立，与原实现一致。
        let prev_ok = start == 0 || !is_word_char(s_bytes[start - 1]);
        let next_ok = end == s_bytes.len() || !is_word_char(s_bytes[end]);
        if prev_ok && next_ok {
            result.push_str(&s[last..start]);
            result.push_str(replacement);
            last = end;
        }
    }

    result.push_str(&s[last..]);
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
