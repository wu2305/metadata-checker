use super::types::{Confidence, RefType, ResolvedRef};

/// 表达式分词结果
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// 标识符（变量、函数名）
    Identifier(String),
    /// 数字字面量
    Number(String),
    /// 字符串字面量（单引号或双引号）
    StringLiteral(String),
    /// 左括号
    LParen,
    /// 右括号
    RParen,
    /// 逗号
    Comma,
    /// 点号
    Dot,
    /// 二元运算符
    BinaryOp(String),
    /// 一元运算符 / 关键字
    UnaryOp(String),
    /// 条件关键字 IF
    If,
    /// 条件关键字 THEN
    Then,
    /// 条件关键字 ELSE
    Else,
    /// 结束
    Eof,
}

/// 表达式 AST 节点
#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    /// 标识符
    Identifier(String),
    /// 成员访问：base.member
    MemberAccess { base: Box<AstNode>, member: String },
    /// 函数调用：name(args)
    FunctionCall { name: String, args: Vec<AstNode> },
    /// 字符串字面量
    StringLiteral(String),
    /// 数字字面量
    NumberLiteral(String),
    /// 布尔字面量
    BooleanLiteral(bool),
    /// 二元运算
    BinaryOp {
        op: String,
        left: Box<AstNode>,
        right: Box<AstNode>,
    },
    /// 一元运算
    UnaryOp { op: String, operand: Box<AstNode> },
    /// 条件表达式：IF condition THEN then_branch ELSE else_branch
    Conditional {
        condition: Box<AstNode>,
        then_branch: Box<AstNode>,
        else_branch: Box<AstNode>,
    },
}

/// 表达式解析诊断
#[derive(Debug, Clone, PartialEq)]
pub struct ExprDiagnostic {
    pub code: String,
    pub message: String,
    pub position: Option<usize>,
}

/// 表达式解析结果
#[derive(Debug, Clone, PartialEq)]
pub struct ExprParseResult {
    pub ast: Option<AstNode>,
    pub refs: Vec<RefType>,
    pub resolved_refs: Vec<ResolvedRef>,
    pub diagnostics: Vec<ExprDiagnostic>,
}

impl ExprParseResult {
    pub fn empty() -> Self {
        Self {
            ast: None,
            refs: Vec::new(),
            resolved_refs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

/// 表达式分词器
struct Tokenizer {
    pos: usize,
    chars: Vec<char>,
}

impl Tokenizer {
    fn new(input: &str) -> Self {
        Self {
            pos: 0,
            chars: input.chars().collect(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_string_literal(&mut self, quote: char) -> Token {
        let mut value = String::new();
        self.advance(); // consume opening quote
        while let Some(c) = self.peek() {
            if c == '\\' {
                self.advance();
                if let Some(next) = self.advance() {
                    value.push(next);
                }
            } else if c == quote {
                self.advance(); // consume closing quote
                break;
            } else {
                value.push(c);
                self.advance();
            }
        }
        Token::StringLiteral(value)
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut value = first.to_string();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                value.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Token::Number(value)
    }

    fn read_identifier(&mut self, first: char) -> Token {
        let mut value = first.to_string();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                value.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let lower = value.to_lowercase();
        match lower.as_str() {
            "if" => Token::If,
            "then" => Token::Then,
            "else" => Token::Else,
            "and" => Token::BinaryOp("AND".to_string()),
            "or" => Token::BinaryOp("OR".to_string()),
            "not" => Token::UnaryOp("NOT".to_string()),
            "is" => Token::BinaryOp("IS".to_string()),
            "in" => Token::BinaryOp("IN".to_string()),
            "between" => Token::BinaryOp("BETWEEN".to_string()),
            "null" => Token::Identifier("NULL".to_string()),
            "true" => Token::Identifier("TRUE".to_string()),
            "false" => Token::Identifier("FALSE".to_string()),
            _ => Token::Identifier(value),
        }
    }

    fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        match self.peek() {
            None => Token::Eof,
            Some('\'') | Some('"') => {
                let quote = self.peek().unwrap();
                self.read_string_literal(quote)
            }
            Some(c) if c.is_ascii_digit() => {
                let first = self.advance().unwrap();
                self.read_number(first)
            }
            Some(c) if c.is_alphabetic() || c == '_' || c == '$' || c == '@' => {
                let first = self.advance().unwrap();
                if first == '$' && self.peek() == Some('{') {
                    // 处理 SuperPage ${model.field} 语法，整体作为一个标识符
                    let mut value = first.to_string();
                    self.advance(); // consume {
                    value.push('{');
                    while let Some(c) = self.peek() {
                        if c == '}' {
                            self.advance(); // consume }
                            value.push('}');
                            break;
                        } else {
                            value.push(c);
                            self.advance();
                        }
                    }
                    Token::Identifier(value)
                } else {
                    self.read_identifier(first)
                }
            }
            Some('+') => {
                self.advance();
                Token::BinaryOp("+".to_string())
            }
            Some('-') => {
                self.advance();
                Token::BinaryOp("-".to_string())
            }
            Some('*') => {
                self.advance();
                Token::BinaryOp("*".to_string())
            }
            Some('/') => {
                self.advance();
                Token::BinaryOp("/".to_string())
            }
            Some('%') => {
                self.advance();
                Token::BinaryOp("%".to_string())
            }
            Some('(') => {
                self.advance();
                Token::LParen
            }
            Some(')') => {
                self.advance();
                Token::RParen
            }
            Some(',') => {
                self.advance();
                Token::Comma
            }
            Some('.') => {
                self.advance();
                Token::Dot
            }
            Some('=') => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Token::BinaryOp("==".to_string())
                } else {
                    Token::BinaryOp("=".to_string())
                }
            }
            Some('!') => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Token::BinaryOp("!=".to_string())
                } else {
                    Token::UnaryOp("!".to_string())
                }
            }
            Some('<') => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Token::BinaryOp("<=".to_string())
                } else if self.peek() == Some('>') {
                    self.advance();
                    Token::BinaryOp("<>".to_string())
                } else {
                    Token::BinaryOp("<".to_string())
                }
            }
            Some('>') => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Token::BinaryOp(">=".to_string())
                } else {
                    Token::BinaryOp(">".to_string())
                }
            }
            Some('&') => {
                self.advance();
                if self.peek() == Some('&') {
                    self.advance();
                    Token::BinaryOp("&&".to_string())
                } else {
                    Token::BinaryOp("&".to_string())
                }
            }
            Some('|') => {
                self.advance();
                if self.peek() == Some('|') {
                    self.advance();
                    Token::BinaryOp("||".to_string())
                } else {
                    Token::BinaryOp("|".to_string())
                }
            }
            Some(c) => {
                self.advance();
                Token::Identifier(c.to_string())
            }
        }
    }
}

/// 轻量递归下降解析器
struct Parser {
    tokenizer: Tokenizer,
    current: Token,
    diagnostics: Vec<ExprDiagnostic>,
}

impl Parser {
    fn new(input: &str) -> Self {
        let mut tokenizer = Tokenizer::new(input);
        let current = tokenizer.next_token();
        Self {
            tokenizer,
            current,
            diagnostics: Vec::new(),
        }
    }

    fn advance(&mut self) -> Token {
        let old = self.current.clone();
        self.current = self.tokenizer.next_token();
        old
    }

    fn parse(&mut self) -> Option<AstNode> {
        if self.current == Token::Eof {
            return None;
        }
        self.parse_conditional()
    }

    fn parse_conditional(&mut self) -> Option<AstNode> {
        let condition = self.parse_or_expr()?;
        if self.current == Token::If || self.current == Token::Then {
            // 处理 IF a THEN b ELSE c 或 a IF b THEN c 形式
            // 实际上常见语法是 IF(cond, trueVal, falseVal) 作为函数
            // 这里处理 IF a THEN b ELSE c 形式
            if self.current == Token::If {
                self.advance(); // consume IF
                let cond = self.parse_or_expr()?;
                if self.current == Token::Then {
                    self.advance();
                }
                let then_branch = self.parse_or_expr()?;
                if self.current == Token::Else {
                    self.advance();
                }
                let else_branch = self.parse_or_expr()?;
                return Some(AstNode::Conditional {
                    condition: Box::new(cond),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                });
            }
        }
        Some(condition)
    }

    fn parse_or_expr(&mut self) -> Option<AstNode> {
        let mut left = self.parse_and_expr()?;
        while matches!(self.current, Token::BinaryOp(ref op) if op == "OR" || op == "||") {
            let op = match self.advance() {
                Token::BinaryOp(op) => op,
                _ => unreachable!(),
            };
            let right = self.parse_and_expr()?;
            left = AstNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn parse_and_expr(&mut self) -> Option<AstNode> {
        let mut left = self.parse_comparison()?;
        while matches!(self.current, Token::BinaryOp(ref op) if op == "AND" || op == "&&") {
            let op = match self.advance() {
                Token::BinaryOp(op) => op,
                _ => unreachable!(),
            };
            let right = self.parse_comparison()?;
            left = AstNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn parse_comparison(&mut self) -> Option<AstNode> {
        let mut left = self.parse_additive()?;
        while matches!(
            self.current,
            Token::BinaryOp(ref op) if matches!(
                op.as_str(),
                "=" | "==" | "!=" | "<>" | "<" | ">" | "<=" | ">=" | "IS" | "IN" | "BETWEEN"
            )
        ) {
            let op = match self.advance() {
                Token::BinaryOp(op) => op,
                _ => unreachable!(),
            };
            let right = self.parse_additive()?;
            left = AstNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn parse_additive(&mut self) -> Option<AstNode> {
        let mut left = self.parse_multiplicative()?;
        while matches!(self.current, Token::BinaryOp(ref op) if op == "+" || op == "-") {
            let op = match self.advance() {
                Token::BinaryOp(op) => op,
                _ => unreachable!(),
            };
            let right = self.parse_multiplicative()?;
            left = AstNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn parse_multiplicative(&mut self) -> Option<AstNode> {
        let mut left = self.parse_unary()?;
        while matches!(self.current, Token::BinaryOp(ref op) if op == "*" || op == "/" || op == "%")
        {
            let op = match self.advance() {
                Token::BinaryOp(op) => op,
                _ => unreachable!(),
            };
            let right = self.parse_unary()?;
            left = AstNode::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<AstNode> {
        if matches!(self.current, Token::UnaryOp(ref op) if op == "NOT" || op == "!" || op == "+") {
            let op = match self.advance() {
                Token::UnaryOp(op) => op,
                _ => unreachable!(),
            };
            let operand = self.parse_unary()?;
            return Some(AstNode::UnaryOp {
                op,
                operand: Box::new(operand),
            });
        }
        self.parse_primary()
    }

    /// 处理标识符或函数调用
    fn handle_identifier_or_function(&mut self, name: String) -> Option<AstNode> {
        // Check for function call
        if self.current == Token::LParen {
            self.advance(); // consume (
            let mut args = Vec::new();
            while self.current != Token::RParen && self.current != Token::Eof {
                if let Some(arg) = self.parse_or_expr() {
                    args.push(arg);
                }
                if self.current == Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            if self.current == Token::RParen {
                self.advance(); // consume )
            } else {
                self.diagnostics.push(ExprDiagnostic {
                    code: "EXPR_PARSE_ERROR".to_string(),
                    message: format!("Expected ')' after function arguments for {}", name),
                    position: Some(self.tokenizer.pos),
                });
            }
            return Some(AstNode::FunctionCall { name, args });
        }
        // Check for member access
        if self.current == Token::Dot {
            let mut current = AstNode::Identifier(name.clone());
            while self.current == Token::Dot {
                self.advance(); // consume .
                if let Token::Identifier(member) = self.current.clone() {
                    self.advance();
                    current = AstNode::MemberAccess {
                        base: Box::new(current),
                        member,
                    };
                } else {
                    self.diagnostics.push(ExprDiagnostic {
                        code: "EXPR_PARSE_ERROR".to_string(),
                        message: format!("Expected identifier after '.', got {:?}", self.current),
                        position: Some(self.tokenizer.pos),
                    });
                    break;
                }
            }
            return Some(current);
        }
        Some(AstNode::Identifier(name))
    }

    fn parse_primary(&mut self) -> Option<AstNode> {
        match self.current.clone() {
            Token::StringLiteral(s) => {
                self.advance();
                Some(AstNode::StringLiteral(s))
            }
            Token::Number(n) => {
                self.advance();
                Some(AstNode::NumberLiteral(n))
            }
            Token::Identifier(name) => {
                self.advance();
                let lower = name.to_lowercase();
                if lower == "true" {
                    return Some(AstNode::BooleanLiteral(true));
                }
                if lower == "false" {
                    return Some(AstNode::BooleanLiteral(false));
                }
                if lower == "null" {
                    return Some(AstNode::Identifier("NULL".to_string()));
                }
                self.handle_identifier_or_function(name)
            }
            Token::If | Token::Then | Token::Else => {
                let name = match self.current {
                    Token::If => "IF".to_string(),
                    Token::Then => "THEN".to_string(),
                    Token::Else => "ELSE".to_string(),
                    _ => unreachable!(),
                };
                self.advance();
                self.handle_identifier_or_function(name)
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_or_expr()?;
                if self.current == Token::RParen {
                    self.advance();
                }
                Some(expr)
            }
            _ => {
                self.diagnostics.push(ExprDiagnostic {
                    code: "EXPR_PARSE_ERROR".to_string(),
                    message: format!("Unexpected token: {:?}", self.current),
                    position: Some(self.tokenizer.pos),
                });
                self.advance();
                None
            }
        }
    }
}

/// 解析表达式为 AST
pub fn parse_expression_ast(expr: &str) -> ExprParseResult {
    // 跳过表达式前导的 = 符号，这在 SuperPage 表达式中很常见
    let expr = expr.trim_start().trim_start_matches('=');
    let mut parser = Parser::new(expr);
    let ast = parser.parse();
    let mut result = ExprParseResult {
        ast,
        refs: Vec::new(),
        resolved_refs: Vec::new(),
        diagnostics: parser.diagnostics,
    };
    if let Some(ref ast) = result.ast {
        extract_refs_from_ast(
            ast,
            &mut result.refs,
            &mut result.resolved_refs,
            &mut result.diagnostics,
        );
    }
    result
}

/// 兼容旧接口：从 AST 中提取 RefType 列表
pub fn parse_expression_refs(expr: &str) -> Vec<RefType> {
    let result = parse_expression_ast(expr);
    // 去重，保留顺序
    let mut seen = std::collections::HashSet::new();
    result
        .refs
        .into_iter()
        .filter(|r| seen.insert(r.clone()))
        .collect()
}

/// 从 MemberAccess 链中提取完整的点分路径
fn get_member_access_path(node: &AstNode) -> Option<String> {
    match node {
        AstNode::Identifier(name) => Some(name.clone()),
        AstNode::MemberAccess { base, member } => {
            get_member_access_path(base).map(|prefix| format!("{}.{}", prefix, member))
        }
        _ => None,
    }
}

/// 从 AST 节点递归提取引用
fn extract_refs_from_ast(
    node: &AstNode,
    refs: &mut Vec<RefType>,
    resolved_refs: &mut Vec<ResolvedRef>,
    diagnostics: &mut Vec<ExprDiagnostic>,
) {
    match node {
        AstNode::Identifier(name) => {
            // 跳过常见的常量、关键字，它们不是引用
            let lower = name.to_lowercase();
            if ["null", "true", "false"].contains(&lower.as_str()) {
                return;
            }
            let ref_type = classify_identifier(name);
            if !matches!(ref_type, RefType::Other(_)) {
                refs.push(ref_type.clone());
                resolved_refs.push(ResolvedRef {
                    ref_type,
                    confidence: Confidence::High,
                    reason: format!("识别为 {}", name),
                    unresolved: false,
                });
            } else {
                // 尝试判断是否是未解析的引用
                if is_likely_unresolved(name) {
                    diagnostics.push(ExprDiagnostic {
                        code: "EXPR_UNRESOLVED_REF".to_string(),
                        message: format!("无法解析的标识符: {}", name),
                        position: None,
                    });
                }
                refs.push(ref_type.clone());
                resolved_refs.push(ResolvedRef {
                    ref_type,
                    confidence: Confidence::Low,
                    reason: format!("无法确认引用类型: {}", name),
                    unresolved: true,
                });
            }
        }
        AstNode::MemberAccess { base, .. } => {
            // 遍历完整的 MemberAccess 链，获取完整路径后统一分类
            if let Some(full_path) = get_member_access_path(node) {
                let ref_type = classify_identifier(&full_path);
                refs.push(ref_type.clone());
                let confidence = match &ref_type {
                    RefType::ModelField(_, _) => Confidence::High,
                    RefType::ComponentValue(_) => Confidence::High,
                    RefType::ComponentProperty(_, _) => Confidence::High,
                    RefType::Param(_) => Confidence::High,
                    RefType::UserProperty(_) => Confidence::High,
                    RefType::SystemVar(_) => Confidence::High,
                    _ => Confidence::Low,
                };
                let reason = match &ref_type {
                    RefType::ModelField(m, f) => format!("模型字段引用: {}.{}", m, f),
                    RefType::ComponentValue(c) => format!("组件值引用: {}", c),
                    RefType::ComponentProperty(c, p) => format!("组件属性引用: {}.{}", c, p),
                    RefType::Param(p) => format!("参数引用: {}", p),
                    RefType::UserProperty(p) => format!("用户属性引用: {}", p),
                    RefType::SystemVar(v) => format!("系统变量引用: {}", v),
                    _ => format!("未知成员访问: {}", full_path),
                };
                resolved_refs.push(ResolvedRef {
                    ref_type,
                    confidence,
                    reason,
                    unresolved: false,
                });
            } else {
                // 回退到递归处理 base
                extract_refs_from_ast(base, refs, resolved_refs, diagnostics);
            }
        }
        AstNode::FunctionCall { name, args } => {
            let lower = name.to_lowercase();
            let supported = [
                "if",
                "concat",
                "sumif",
                "countif",
                "user_ingroup",
                "today",
                "uuid",
                "isnull",
                "round",
                "sum",
                "avg",
                "max",
                "min",
                "count",
                "len",
                "trim",
                "upper",
                "lower",
            ];
            if !supported.contains(&lower.as_str()) {
                diagnostics.push(ExprDiagnostic {
                    code: "EXPR_UNSUPPORTED_FUNCTION".to_string(),
                    message: format!("不支持的函数: {}", name),
                    position: None,
                });
            }
            for arg in args {
                extract_refs_from_ast(arg, refs, resolved_refs, diagnostics);
            }
        }
        AstNode::BinaryOp { left, right, .. } => {
            extract_refs_from_ast(left, refs, resolved_refs, diagnostics);
            extract_refs_from_ast(right, refs, resolved_refs, diagnostics);
        }
        AstNode::UnaryOp { operand, .. } => {
            extract_refs_from_ast(operand, refs, resolved_refs, diagnostics);
        }
        AstNode::Conditional {
            condition,
            then_branch,
            else_branch,
        } => {
            extract_refs_from_ast(condition, refs, resolved_refs, diagnostics);
            extract_refs_from_ast(then_branch, refs, resolved_refs, diagnostics);
            extract_refs_from_ast(else_branch, refs, resolved_refs, diagnostics);
        }
        // 字面量不产生引用
        AstNode::StringLiteral(_) | AstNode::NumberLiteral(_) | AstNode::BooleanLiteral(_) => {}
    }
}

/// 判断标识符是否可能是未解析的引用
fn is_likely_unresolved(name: &str) -> bool {
    // 排除常见的函数名和关键字
    let lower = name.to_lowercase();
    let known_functions = [
        "if",
        "concat",
        "sumif",
        "countif",
        "user_ingroup",
        "today",
        "uuid",
        "isnull",
        "round",
        "sum",
        "avg",
        "max",
        "min",
        "count",
        "len",
        "trim",
        "upper",
        "lower",
        "abs",
        "mod",
        "power",
        "sqrt",
    ];
    if known_functions.contains(&lower.as_str()) {
        return false;
    }
    // 如果看起来像标识符但无法分类，可能是未解析的
    name.chars()
        .next()
        .map(|c| c.is_alphabetic() || c == '_' || c == '$' || c == '@')
        .unwrap_or(false)
}

/// 分类单个标识符
fn classify_identifier(token: &str) -> RefType {
    let token = token.trim();

    // 处理 SuperPage ${model.field} 语法
    if token.starts_with("${") && token.ends_with("}") {
        let inner = &token[2..token.len() - 1];
        let parts: Vec<&str> = inner.split('.').collect();
        if parts.len() >= 2 {
            let field = parts[1..].join(".");
            return RefType::ModelField(parts[0].to_string(), field);
        }
        return RefType::ModelField(inner.to_string(), String::new());
    }

    if let Some(stripped) = token.strip_prefix('$') {
        let parts: Vec<&str> = stripped.split('.').collect();
        if parts.len() >= 2 {
            return RefType::UserProperty(stripped.to_string());
        }
        return RefType::SystemVar(token.to_string());
    }

    let parts: Vec<&str> = token.split('.').collect();

    if parts.len() >= 2 {
        let first = parts[0];
        if first.starts_with("model") {
            let field = parts[1..].join(".");
            return RefType::ModelField(first.to_string(), field);
        }
        if parts.last() == Some(&"value") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.last() == Some(&"step") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.len() >= 3 && parts[1] == "checked" && parts[2] == "value" {
            return RefType::ComponentProperty(first.to_string(), "checked.value".to_string());
        }
        return RefType::Other(token.to_string());
    }

    if token.starts_with("param") {
        return RefType::Param(token.to_string());
    }

    // 排除常见的常量、关键字和函数名
    let lower = token.to_lowercase();
    if ["null", "true", "false"].contains(&lower.as_str()) {
        return RefType::Other(token.to_string());
    }

    if !token
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true)
    {
        return RefType::ComponentValue(token.to_string());
    }

    RefType::Other(token.to_string())
}
