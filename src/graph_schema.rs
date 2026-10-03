//! 图 Schema 契约：项目图的全部合法形状，及读图者可以（不可以）据此下的结论。
//!
//! 这张表是**唯一真源**（设计见 `docs/specs/2026-10-02-graph-schema-contract-design.md`）：
//! - LLM 侧：`--graph-schema` 与 `--gql` 的 long help 都从这里生成；
//! - 测试侧：`tests/graph_schema_tests.rs` 在扫描出的真实图上逐元素核对；
//!   `docs/reference/graph-schema.md` 由 [`to_markdown`] 生成并比对漂移。
//!
//! 本批（S1）只**记录现状**，不改图结构，也不在写入路径上校验。表里的端点组合与
//! `meta` 键来自夹具图实测加 `scanner/` 写入点核对；`required = true` 仅用于写入点
//! 无条件构造的键（Action、Condition 的 `meta`）。S2 在启用写入校验前必须逐写入点
//! 复核端点组合，不能直接把夹具观测当成穷举。
//!
//! 表的形状：每个节点类型有 id 文法、`meta` 键（名 + JSON 类型 + 是否可空 + 含义，`meta_open`
//! 的行允许出现未列出的键）；每种边有端点组合、`field_path` 语义、`meta` 键、**产生它的写入路径与规则**
//! （`producers`，测试核对引用的文件与函数还在）以及「缺席不代表没有」的情形。示例查询、标签与固有属性
//! 也在表里，`--graph-schema` 的 JSON / Markdown 与 `--gql` help 都从同一份数据渲染。
//!
//! 核对范围：夹具图逐元素核对；设置了 `METADATA_CHECKER_REAL_PROJECT_DIR` 时对真实语料同样核对
//! （夹具覆盖不到的分支只有真实语料能暴露）。页面局部 id（`model:<PAGE>|<local>`）只出现在
//! ownership 绑定的 redb 图里，`.grafeo` 图（`--gql` 读的那张）目前看不到，所以那几种 id 形态
//! 只按 `graph_identity.rs` 记录，没有被一致性测试核对。
//!
//! 与存储层版本无关：[`GRAPH_SCHEMA_CONTRACT_VERSION`] 描述本契约，不是 redb 的
//! `fact_schema_version`；S3 引入图内新事实时才会递增。

use crate::graph::{EdgeType, NodeType};
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// 契约版本。S3（字段级血缘边）与 S4（诊断入图）会让它变成 2。
pub const GRAPH_SCHEMA_CONTRACT_VERSION: u32 = 1;

/// `meta` 值的 JSON 类型。`Any` 只用于确实多态的键（见各键描述）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaValueType {
    String,
    Bool,
    Number,
    Array,
    Object,
    Any,
}

/// `meta` 里的一个键。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MetaKeySchema {
    pub name: &'static str,
    pub value_type: MetaValueType,
    /// 值是否可以为 JSON `null`（键本身存在，值为空）。
    pub nullable: bool,
    /// 写入点是否无条件写出该键。
    pub required: bool,
    pub description: &'static str,
}

impl MetaKeySchema {
    /// 同一个键允许 `null` 值。
    const fn nullable(self) -> Self {
        Self {
            nullable: true,
            ..self
        }
    }
}

/// 边的起止节点类型组合。
#[derive(Debug, Clone, Serialize)]
pub struct EndpointPair {
    pub from: NodeType,
    pub to: NodeType,
}

/// 边的 `field_path` 属性是否出现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldPathUse {
    /// 从不设置。
    Never,
    /// 总是设置。
    Always,
    /// 取决于端点组合或写入分支，见 `field_path_meaning`。
    Sometimes,
}

/// 节点类型的契约行。
#[derive(Debug, Clone, Serialize)]
pub struct NodeTypeSchema {
    pub node_type: NodeType,
    pub summary: &'static str,
    /// 允许的 id 前缀（冒号之前的部分）。
    pub id_prefixes: &'static [&'static str],
    pub id_formats: &'static [&'static str],
    /// `path` 属性的含义。
    pub path_meaning: &'static str,
    pub meta_keys: &'static [MetaKeySchema],
    /// `meta` 是否开放：`true` 表示除已列出的键外还会出现别的键（例如表字段的 `meta` 是
    /// `.tbl` 维度定义的原样拷贝），写入校验不得据此拒绝未列出的键。
    pub meta_open: bool,
    pub notes: &'static str,
}

/// 产生某种边的一条写入路径。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Producer {
    /// `src/` 之下的文件与函数，形如 `scanner/spg.rs::process_spg_file_from_value_with_identity`
    /// （不写行号：行号会漂移；测试核对文件与函数名都还在）。
    pub path: &'static str,
    /// 该路径在什么条件下写出这种边。
    pub rule: &'static str,
}

/// 边类型的契约行。
#[derive(Debug, Clone, Serialize)]
pub struct EdgeTypeSchema {
    pub edge_type: EdgeType,
    pub summary: &'static str,
    /// 扫描器当前是否会写出这种边。`false` 表示枚举里有、但没有任何写入点。
    pub emitted: bool,
    pub endpoints: &'static [EndpointPair],
    pub field_path: FieldPathUse,
    pub field_path_meaning: &'static str,
    pub meta_keys: &'static [MetaKeySchema],
    /// 产生这种边的写入路径与规则。会产生的边必须至少有一条。
    pub producers: &'static [Producer],
    /// 这种边缺席时，「没有该关系」是否成立；不成立时写明原因。
    pub absent_when: &'static str,
}

/// 解释规则：由图里的事实能下什么结论、不能下什么。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct InterpretationRule {
    pub id: &'static str,
    pub rule: &'static str,
}

/// 图库里的标签与固有属性（节点 / 边本身的列，不是 `meta` 里的键）。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GraphLabels {
    /// 项目节点的标签。
    pub project_node: &'static str,
    pub node_properties: &'static [&'static str],
    pub edge_properties: &'static [&'static str],
    /// 图库里同时存在的内部记录标签，查询时不要碰。
    pub internal_label: &'static str,
}

/// 一条示例查询。每条都由测试在夹具图上真实执行，保证示例不会过期。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct QueryExample {
    pub title: &'static str,
    pub gql: &'static str,
}

/// 完整契约。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GraphSchema {
    pub contract_version: u32,
    pub labels: GraphLabels,
    pub node_types: &'static [NodeTypeSchema],
    pub edge_types: &'static [EdgeTypeSchema],
    pub interpretation_rules: &'static [InterpretationRule],
    pub examples: &'static [QueryExample],
    pub dialect_notes: &'static [&'static str],
}

/// 可选键（写入点有条件地写出）。
const fn opt(
    name: &'static str,
    value_type: MetaValueType,
    description: &'static str,
) -> MetaKeySchema {
    MetaKeySchema {
        name,
        value_type,
        nullable: false,
        required: false,
        description,
    }
}

/// 必需键（写入点无条件写出）。
const fn req(
    name: &'static str,
    value_type: MetaValueType,
    description: &'static str,
) -> MetaKeySchema {
    MetaKeySchema {
        name,
        value_type,
        nullable: false,
        required: true,
        description,
    }
}

const fn pair(from: NodeType, to: NodeType) -> EndpointPair {
    EndpointPair { from, to }
}

const fn producer(path: &'static str, rule: &'static str) -> Producer {
    Producer { path, rule }
}

use MetaValueType::{Any, Array, Bool, Number, Object, String as Str};

// ---------------------------------------------------------------- 节点

const NODE_TYPES: &[NodeTypeSchema] = &[
    NodeTypeSchema {
        node_type: NodeType::Page,
        summary: "A SuperPage file (.spg), or a page that another page refers to.",
        id_prefixes: &["page"],
        id_formats: &["page:<project-relative path>"],
        path_meaning: "The .spg path. A referenced page is created from the link target even when no file exists there, so a Page node is a dangling reference unless a scanned file backs it (see the dangling-page rule); a missing Contains edge alone does not tell, since a scanned page with no canvas has none.",
        meta_keys: &[],
        meta_open: false,
        notes: "No meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Component,
        summary: "A component inside a page (including the canvas root and dialogs).",
        id_prefixes: &["comp"],
        id_formats: &["comp:<page path>|<component id>"],
        path_meaning: "The page path the component lives in.",
        meta_keys: &[
            opt("component_type", Str, "The component's type string."),
            opt(
                "json_path",
                Str,
                "Location of the component inside the page JSON.",
            ),
            opt(
                "parent_id",
                Str,
                "Id of the enclosing component; absent on the root.",
            ),
            opt(
                "properties",
                Object,
                "Object with the extracted properties (exp, value, visibleCondition, disableCondition, submitField, ...).",
            ),
            opt("dataSet", Str, "Bound data set, on data-bound components."),
            opt(
                "source",
                Str,
                "Data source reference, on data-bound components.",
            ),
            opt(
                "data_context_component_id",
                Str,
                "Component that supplies the data context.",
            ),
            opt(
                "data_context_dataSet",
                Str,
                "Data set of that data context.",
            ),
            opt(
                "data_context_json_path",
                Str,
                "JSON path of that data context.",
            ),
            opt("data_context_source", Str, "Source of that data context."),
        ],
        meta_open: false,
        notes: "Some components (for example the canvas root, or a dialog that is only the target of a showDialog action) have no meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Model,
        summary: "A data model: a physical table (.tbl), a dataflow, a model referenced by a page, or an output table of a dataflow.",
        id_prefixes: &["model"],
        id_formats: &[
            "model:<name>",
            "model:<page path>|<local name> (ownership-bound graphs only; see notes)",
        ],
        path_meaning: "Depends on where the node came from: the .tbl file path for a scanned table; the .spg path for a dataflow embedded in a page; the path as declared by the referring file for a referenced table (for example $DATA:/dir/x.tbl or ../x.tbl, not resolved to a file); '<name>.tbl' for an output table or an unresolved model. A declared or placeholder path is not evidence that the file exists.",
        meta_keys: &[
            opt(
                "modelType",
                Str,
                "Model kind: App or DataFlow (a scanned .tbl), PhysicalTable (output or referenced table), dwtable (a page source bound to a table), DataFlowDependency (listed in a dataflow's depends).",
            ),
            opt("sourcePath", Str, "Declared table path."),
            opt(
                "dimensions",
                Array,
                "Array of field definitions (name, dbfield, dataType, isDimension, length).",
            ),
            opt(
                "embeddedIn",
                Str,
                "Page name, on a dataflow embedded in a page.",
            ),
            opt("aliasMap", Object, "Dataflow: alias to table mapping."),
            opt(
                "internalDeps",
                Object,
                "Dataflow: dependencies between internal nodes.",
            ),
            opt("nodeFields", Object, "Dataflow: fields per internal node."),
            opt("nodeFilters", Object, "Dataflow: filters per internal node."),
            opt(
                "nodeJoinConditions",
                Object,
                "Dataflow: join conditions per internal node.",
            ),
            opt(
                "nodeTablePaths",
                Object,
                "Dataflow: table path per internal node.",
            ),
            opt("nodeTypes", Object, "Dataflow: type per internal node."),
            opt(
                "nodeUnionMaps",
                Object,
                "Dataflow: union mappings per internal node.",
            ),
        ],
        meta_open: false,
        notes: "In the .grafeo graph that --gql reads, model names are global: a page source called model1 on two pages collapses into one node model:model1, and a page source named like a table is the same node as the table. Ownership-bound graphs (redb with a project binding) instead give page-local models the id model:<page path>|<local name>; ownership scanning is not wired for Grafeo, so GQL does not see that form today.",
    },
    NodeTypeSchema {
        node_type: NodeType::Field,
        summary: "A table field, or a non-table symbol: a page parameter, a user property or a system variable.",
        id_prefixes: &["field", "param", "user", "system"],
        id_formats: &[
            "field:<model>.<field>",
            "field:<model>.* (stands for all fields of the model; target of loadData-style writes)",
            "field:<page path>|<model>.<field> (ownership-bound graphs only; see the Model notes)",
            "param:<page path>|<param name>",
            "param:<page name>/<param name> (parameter of a referenced page)",
            "user:<namespace>.<name> (the part after $; the namespace is usually user, but =$project.name gives user:project.name)",
            "system:<name>",
        ],
        path_meaning: "The path of the model the field belongs to (the .tbl path for field:, or the .spg path for a field of a dataflow embedded in a page), the page path for param:, and 'system' for user: and system:.",
        meta_keys: &[
            opt(
                "kind",
                Str,
                "param | user_property | system_var on non-table symbols; implicit on the model's totalRowCount__ field.",
            ),
            opt("name", Str, "Display name of a table field."),
            opt("dataType", Str, "Field data type."),
            opt("dbfield", Str, "Physical column name."),
            opt("isDimension", Bool, "Whether the field is a dimension."),
            opt("length", Number, "Declared length."),
            opt("description", Str, "Field description."),
            opt("inputField", Str, "Dataflow: upstream input field."),
            opt(
                "source_input_field",
                Str,
                "Dataflow: upstream input field (source form).",
            ),
            opt(
                "originalField",
                Str,
                "Dataflow: original field behind an alias.",
            ),
            opt(
                "originalNode",
                Str,
                "Dataflow: node the original field belongs to.",
            ),
            opt("exp", Str, "Dataflow: computed-field expression."),
            opt(
                "source_expr",
                Str,
                "Dataflow: expression text the field came from.",
            ),
            opt(
                "source_expr_models",
                Array,
                "Names of the models the expression refers to (strings), when it refers to any.",
            ),
            opt("dimensionPath", Str, "Dataflow: dimension path."),
            opt(
                "businessDesc",
                Str,
                "Business description written on the field definition.",
            ),
            opt("fieldRole", Str, "Role of the field as authored."),
            opt("hidden", Bool, "Field is hidden, as authored."),
            opt("isPrimaryKey", Bool, "Field is a primary key, as authored."),
            opt("defaultValueExp", Str, "Default-value expression, as authored."),
            opt("aggType", Str, "Aggregation type, as authored."),
            opt("isAggFun", Bool, "Whether the field is an aggregate, as authored."),
            opt("periodType", Str, "Period type, as authored."),
            opt("dateFormat", Str, "Date format, as authored."),
            opt("dateKeyValueFormat", Str, "Date key/value format, as authored."),
            opt("displayFormat", Str, "Display format, as authored."),
            opt("displayDimension", Str, "Display dimension, as authored."),
            opt("textField", Str, "Text field, as authored."),
            opt("decimal", Number, "Decimal places, as authored."),
            opt("precision", Number, "Precision, as authored."),
            opt("geoType", Str, "Geo type, as authored."),
            opt("extractable", Bool, "Extractable flag, as authored."),
            opt("isAutoInc", Bool, "Auto-increment flag, as authored."),
            opt("newborn", Bool, "Newborn flag, as authored."),
            opt(
                "isOriginalFieldInvalid",
                Bool,
                "Whether the original field is invalid, as authored.",
            ),
            opt("fileModifyTimeField", Str, "File modify-time field, as authored."),
            opt(
                "fieldStorageInfo",
                Object,
                "Storage details of the field, as authored.",
            ),
            opt(
                "modifiedInfo",
                Object,
                "Modification details of the field, as authored.",
            ),
            opt("labels", Array, "Labels on the field, as authored."),
        ],
        meta_open: true,
        notes: "For a table field, meta is a copy of the field's dimension definition in the .tbl, plus the source_* keys the scanner adds, so keys other than the ones listed can occur: the platform adds dimension properties over time. field:<model>.* stands for 'all fields of the model'. Most table fields have no meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Action,
        summary: "An action configured on a component (click handler and similar).",
        id_prefixes: &["action"],
        id_formats: &["action:<page path>|<component id>|<action id>"],
        path_meaning: "The page path the action lives in.",
        meta_keys: &[
            req(
                "triggerType",
                Str,
                "Event that fires the action, for example click; may be an empty string.",
            ),
            req(
                "condition",
                Any,
                "Raw condition value as authored, or null (null on every action seen so far).",
            )
            .nullable(),
            req("conditionExp", Str, "Condition expression, or null.").nullable(),
            req(
                "waitPrev",
                Str,
                "Which earlier action this one waits for: 'nowait' or a '<component>.<action>' reference; null when not set.",
            )
            .nullable(),
        ],
        meta_open: false,
        notes: "The action's own type (submitData, link, ...) is not stored in meta; it shows in the edges the action has (see each Action* edge type) and in the node name (<actionType>:<action id>).",
    },
    NodeTypeSchema {
        node_type: NodeType::Condition,
        summary: "An expression with a semantic role: a component's exp / visibleCondition / disableCondition, an action's condition or conditionExp, or a model filter clause.",
        id_prefixes: &["cond"],
        id_formats: &[
            "cond:<page path>|<component id>#<property>#<n> (component expression; n is the occurrence index)",
            "cond:<page path>|<component id>#<action id>#conditionExp (action conditionExp)",
            "cond:<page path>|<component id>#<action id>#condition (action condition)",
            "cond:<page path>|<source id>#filter#<n>#exp (model filter expression)",
            "cond:<page path>|<source id>#filter#<n>#clause (model filter clause)",
        ],
        path_meaning: "The page path the condition lives in.",
        meta_keys: &[
            req(
                "condition_type",
                Str,
                "What kind of expression this is (for example FieldExp, VisibleCondition, SourceFilterExp).",
            ),
            req(
                "effect_type",
                Str,
                "What the expression controls (for example Compute, Show, Filter).",
            ),
            req("subject_type", Str, "Type of the owner."),
            req("raw_expr", Str, "The expression as written."),
            req(
                "normalized_expr",
                Str,
                "The expression after symbol normalisation.",
            ),
            req("json_path", Str, "Location inside the page JSON."),
            req("owner_type", Str, "Type of the owner."),
            req("owner_id", Str, "Id of the owner within the page."),
            req(
                "referenced_symbols",
                Array,
                "Array of symbols (model:..., component:..., param:...) the expression references.",
            ),
        ],
        meta_open: false,
        notes: "Model filter clauses are Condition nodes owned by the model (json_path starts with sources[...].filter); read raw_expr for the filter fields.",
    },
];

// ---------------------------------------------------------------- 边

/// SPG 页面写图的总入口：几乎所有页面侧的边都从这里产生。
const SPG: &str = "scanner/spg.rs::process_spg_file_from_value_with_identity";
/// 表达式里的模型字段读取最终经由这个函数落边。
const SPG_READ: &str = "scanner/spg.rs::add_model_read_with_scope";
/// 模型字段写入（含 action 的 validate / load）最终经由这个函数落边。
const SPG_WRITE: &str = "scanner/spg.rs::add_model_write_with_scope";
const TBL: &str = "scanner/tbl.rs::process_tbl_file_from_string";

/// 由组件产生的边共有的来源键。
const ACTOR_KEYS_COMPONENT: &[MetaKeySchema] = &[
    opt("actor_kind", Str, "component."),
    opt("actor_id", Str, "Id of the component within its page."),
    opt("operation", Str, "The edge type name."),
    opt(
        "reason",
        Str,
        "Human-readable sentence describing why the edge exists.",
    ),
    opt(
        "source_expr",
        Str,
        "Expression text the edge was derived from.",
    ),
    opt("target_model", Str, "Model the edge refers to."),
    opt("target_field", Str, "Field the edge refers to."),
];

const NO_META: &[MetaKeySchema] = &[];

const EDGE_TYPES: &[EdgeTypeSchema] = &[
    EdgeTypeSchema {
        edge_type: EdgeType::Contains,
        summary: "Structural containment: page contains component, component contains component, model contains field.",
        emitted: true,
        endpoints: &[
            pair(NodeType::Page, NodeType::Component),
            pair(NodeType::Component, NodeType::Component),
            pair(NodeType::Model, NodeType::Field),
        ],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not set.",
        meta_keys: NO_META,
        producers: &[
            producer(
                SPG,
                "Second pass over the page's components: page -> every component, and parent component -> nested component (one Component parent each).",
            ),
            producer(
                SPG,
                "A dataflow embedded in the page: model -> each of its dimension fields; a model with a filter also contains the implicit field totalRowCount__ (the totalRowCount__ edge is missing when the model node does not exist yet at that point, see DependsOn).",
            ),
            producer(
                "scanner/spg.rs::ensure_model_field_with_scope",
                "Every model.field a page expression touches: model -> field.",
            ),
            producer(
                TBL,
                "A .tbl: model -> each dimension field; the output table (dbTableName) also contains the same dimension fields.",
            ),
        ],
        absent_when: "Complete for what the scanner parsed.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::Triggers,
        summary: "A component owns an action (the action fires from that component).",
        emitted: true,
        endpoints: &[pair(NodeType::Component, NodeType::Action)],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not set.",
        meta_keys: NO_META,
        producers: &[producer(
            SPG,
            "Actions pass: every action of every component gets component -> action.",
        )],
        absent_when: "Complete for what the scanner parsed.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::Reads,
        summary: "A component's expression reads a model field.",
        emitted: true,
        endpoints: &[
            pair(NodeType::Component, NodeType::Field),
            pair(NodeType::Component, NodeType::Model),
        ],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is read; just <model> when the expression names the model without a field (a bare ${model1}), which goes with a Model target.",
        meta_keys: &[
            opt("actor_kind", Str, "component."),
            opt("actor_id", Str, "Component id."),
            opt("operation", Str, "Reads."),
            opt("reason", Str, "Why the edge exists."),
            opt(
                "json_path",
                Str,
                "Where in the page JSON the expression is.",
            ),
            opt("source_expr", Str, "The expression text."),
            opt(
                "source_field",
                Str,
                "The property holding the expression (exp, value, ...).",
            ),
            opt("target_model", Str, "Model read."),
            opt("target_field", Str, "Field read."),
            opt(
                "bare_symbol",
                Str,
                "Set when the reference had no model prefix and was resolved through the data context.",
            ),
            opt("resolution", Str, "How a bare symbol was resolved."),
            opt("target_model_path", Str, "Path of the resolved model."),
            opt(
                "data_context_component_id",
                Str,
                "Data-context component used for resolution.",
            ),
            opt("data_context_dataSet", Str, "Data set used for resolution."),
            opt(
                "data_context_json_path",
                Str,
                "JSON path used for resolution.",
            ),
            opt("data_context_source", Str, "Source used for resolution.").nullable(),
        ],
        producers: &[
            producer(
                SPG,
                "A component expression (exp, value, visibleCondition, ...) that references model.field, through add_model_read_with_scope.",
            ),
            producer(
                SPG_READ,
                "Writes the Component -> Model edge and, when the field name is known, the Component -> Field edge; a local model whose source maps to a physical table gets the same pair again on the physical table.",
            ),
            producer(
                SPG,
                "A bare symbol in a component's value that is not a model, component or parameter, inside an inherited data context: resolved against the context's data set.",
            ),
        ],
        absent_when: "A component whose expressions reference no model or field has no Reads edge. A component bound through a data context may only show a Reads edge with a bare_symbol resolution.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::Writes,
        summary: "A component's submitField binds the component to a model field it writes.",
        emitted: true,
        endpoints: &[pair(NodeType::Component, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is written.",
        meta_keys: ACTOR_KEYS_COMPONENT,
        producers: &[producer(
            SPG,
            "A component with properties.submitField of the form model.field: Component -> Model through add_model_write_with_scope (which also adds the field-level FieldWrite edge).",
        )],
        absent_when: "Only components with a submitField produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::FieldWrite,
        summary: "Field-level edge written next to a model-level one by the same actor: a component's submitField, or an action. Despite the name it is not always a write: meta.operation says which (Writes, ActionWrites, ActionValidates, ActionLoadsData).",
        emitted: true,
        endpoints: &[
            pair(NodeType::Action, NodeType::Field),
            pair(NodeType::Component, NodeType::Field),
        ],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is written, validated or loaded; <model>.* for ActionLoadsData.",
        meta_keys: &[
            opt("actor_kind", Str, "component | action."),
            opt("actor_id", Str, "Id of the actor."),
            opt(
                "operation",
                Str,
                "Writes (component submitField), ActionWrites, ActionValidates or ActionLoadsData: the model-level edge type written next to this one. Filter on it before calling the edge a write.",
            ),
            opt("reason", Str, "Why the edge exists."),
            opt("trigger", Str, "Action trigger, on action actors."),
            opt(
                "source_expr",
                Str,
                "Value expression, when the write has one.",
            ),
            opt("target_model", Str, "Model written."),
            opt(
                "target_field",
                Str,
                "Field written; absent on ActionLoadsData.",
            ),
        ],
        producers: &[producer(
            SPG_WRITE,
            "Called for every model-level Writes / ActionWrites / ActionValidates / ActionLoadsData edge; when the field name is non-empty it adds Actor -> Field with the same meta, on the local model and again on the physical table.",
        )],
        absent_when: "A write whose field cannot be resolved has only the model-level edge.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionWrites,
        summary: "An action writes to a model (submitData, insertData, updateData, deleteData, ...).",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is written.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionWrites."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("source_expr", Str, "Value expression."),
            opt("target_model", Str, "Model written."),
            opt("target_field", Str, "Field written."),
        ],
        producers: &[
            producer(
                SPG,
                "submitData: for each component in the submit range that has a submitField, one edge per model.field.",
            ),
            producer(
                SPG,
                "updateData / insertData / deleteData: one edge per configured field value of the action's data set.",
            ),
        ],
        absent_when: "Action types that write nothing have no such edge.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionReads,
        summary: "An action reads a model or field (as a query input, a link parameter value or a set-parameter value).",
        emitted: true,
        endpoints: &[
            pair(NodeType::Action, NodeType::Field),
            pair(NodeType::Action, NodeType::Model),
        ],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is read; just <model> when the expression names the model without a field (a bare ${model1}), which goes with a Model target.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "Reads."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("source_expr", Str, "Expression text."),
            opt("target_model", Str, "Model read."),
            opt("target_field", Str, "Field read."),
        ],
        producers: &[
            producer(
                SPG,
                "An expression-valued field of an insertData / updateData / deleteData action that references model.field.",
            ),
            producer(
                SPG,
                "A link(app) action's data entries: expressions in parameter values.",
            ),
            producer(
                SPG,
                "A setParamValue action's params: expressions in parameter values.",
            ),
        ],
        absent_when: "Only expressions that reference a model or field produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionNavigates,
        summary: "A link action opens another page.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Page)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Project-relative path of the target page.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionNavigates."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("target_model", Str, "Target page name."),
        ],
        producers: &[producer(
            SPG,
            "A link action with targetType app whose path is an index into referenceResources that resolves to a .spg page path: the target Page node is upserted, then Action -> Page.",
        )],
        absent_when: "A link whose target cannot be resolved to a page path (index out of range, an absolute path, a URI, an unknown $-prefix, a target that is not .spg) produces no edge and no Page node, and nothing in the graph marks the gap. A link whose path resolves but whose file does not exist still produces the edge, to a Page node that no scanned file backs (no file_state record). Absence is not proof there is no navigation. The query-time diagnostic UNRESOLVED_PAGE_NAVIGATION only fires for a target node that is missing from the graph, which this scanner never leaves, so it does not cover either case.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::PassesParam,
        summary: "A link action passes a value into a parameter of the target page.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "The value expression being passed (for example =model1.fieldA), not a model.field path.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "PassesParam."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("target_field", Str, "Parameter name."),
            opt("source_expr", Str, "Value expression."),
        ],
        producers: &[producer(
            SPG,
            "A link(app) action with a resolved target page: one Action -> param:<page name>/<param> edge per data entry.",
        )],
        absent_when: "Only link actions that carry parameters and whose target resolves produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionSetsParam,
        summary: "A setParamValue action sets a page parameter.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "The value expression being assigned (for example =model1.fieldB).",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionSetsParam."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("target_field", Str, "Parameter name."),
            opt("source_expr", Str, "Value expression."),
        ],
        producers: &[producer(
            SPG,
            "A setParamValue action: one Action -> param:<page path>|<param> edge per entry of the action's params.",
        )],
        absent_when: "Only setParamValue actions produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionControlsComponent,
        summary: "An action controls a UI component: opens or closes a dialog, shows or hides a component, switches a panel.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Component)],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not set.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionControlsComponent."),
            opt("trigger", Str, "Event that fires the action."),
            opt(
                "reason",
                Str,
                "Says whether it opens, closes, shows, hides or switches.",
            ),
            opt(
                "target_component",
                Str,
                "Id of the component shown or hidden, on showComponent / hideComponent.",
            ),
            opt("dialog", Str, "Dialog id, for showDialog."),
            opt("panel", Str, "Panel id, for switchPanel; null when unset.").nullable(),
            opt("panelbook", Str, "Panel book id, for switchPanel."),
        ],
        producers: &[
            producer(
                SPG,
                "showComponent / hideComponent: one edge per entry of the action's target components.",
            ),
            producer(SPG, "showDialog: the dialog component named by the action."),
            producer(
                SPG,
                "closeDialog: points at the component the action sits on (the dialog being closed).",
            ),
            producer(SPG, "switchPanel: the action's panel book."),
        ],
        absent_when: "Only UI-control action types produce it. A showComponent / hideComponent whose target id is not a component of the page produces no edge (the scanner does not create the target, and the store drops an edge to a missing node), and nothing in the graph marks the gap.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionValidates,
        summary: "A validateData action validates a model field.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is validated.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionValidates."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt("source_expr", Str, "Expression text."),
            opt("target_model", Str, "Model validated."),
            opt("target_field", Str, "Field validated."),
        ],
        producers: &[producer(
            SPG,
            "validateData: for each component in the validate range that has a submitField, one edge per model.field (through add_model_write_with_scope, so a FieldWrite edge with operation ActionValidates comes with it).",
        )],
        absent_when: "Only validateData actions over components with a submitField produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionLoadsData,
        summary: "A loadData / resetData style action loads or resets a model, or the components it names.",
        emitted: true,
        endpoints: &[
            pair(NodeType::Action, NodeType::Model),
            pair(NodeType::Action, NodeType::Component),
        ],
        field_path: FieldPathUse::Sometimes,
        field_path_meaning: "Action -> Model: <model>.* (the whole model). Action -> Component: not set.",
        meta_keys: &[
            opt("actor_kind", Str, "action."),
            opt("actor_id", Str, "Action id."),
            opt("operation", Str, "ActionLoadsData."),
            opt("trigger", Str, "Event that fires the action."),
            opt("reason", Str, "Why the edge exists."),
            opt(
                "target_model",
                Str,
                "Model loaded, on Action -> Model edges.",
            ),
            opt(
                "target_component",
                Str,
                "Component loaded or reset, on Action -> Component edges.",
            ),
        ],
        producers: &[
            producer(
                SPG,
                "resetData / newData / refreshModels / refreshData / loadData with a data set, or with components whose submitField names a model: Action -> Model with field <model>.* (through add_model_write_with_scope, so a FieldWrite edge to field:<model>.* with operation ActionLoadsData comes with it).",
            ),
            producer(
                SPG,
                "The same action types with only submit components that carry no submitField: Action -> each named Component, no field_path.",
            ),
        ],
        absent_when: "Only load / reset / refresh action types produce it. A submit component id that is not a component of the page produces no Component edge, and nothing in the graph marks the gap.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::EmbedsPage,
        summary: "A component embeds another page (composition, not a user navigation).",
        emitted: true,
        endpoints: &[pair(NodeType::Component, NodeType::Page)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Project-relative path of the embedded page.",
        meta_keys: NO_META,
        producers: &[producer(
            SPG,
            "An embedsuperpage component whose resPath is an index into referenceResources that resolves to a .spg page path: the target Page node is upserted, then Component -> Page.",
        )],
        absent_when: "An embed whose target cannot be resolved produces no edge and nothing in the graph marks the gap; one whose path resolves but whose file does not exist produces the edge to a Page node that no scanned file backs (no file_state record).",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DependsOn,
        summary: "Overloaded edge. operation = DependsOn: an expression depends on an upstream symbol (component value or property, parameter, user property, system variable, model field). operation = Conditions: a Condition node points at the owner it belongs to (Condition -> owner).",
        emitted: true,
        endpoints: &[
            pair(NodeType::Component, NodeType::Component),
            pair(NodeType::Component, NodeType::Field),
            pair(NodeType::Condition, NodeType::Action),
            pair(NodeType::Condition, NodeType::Component),
            pair(NodeType::Condition, NodeType::Field),
            pair(NodeType::Condition, NodeType::Model),
        ],
        field_path: FieldPathUse::Always,
        field_path_meaning: "For operation = DependsOn: the referenced symbol (for example comp:input2.value, param:param1, $user.id). For operation = Conditions: the json_path of the condition.",
        meta_keys: &[
            opt("actor_kind", Str, "component | condition."),
            opt("actor_id", Str, "Component id or condition id."),
            opt("operation", Str, "DependsOn or Conditions."),
            opt(
                "reason",
                Str,
                "Says whether the condition belongs to its owner or depends on a symbol.",
            ),
            opt("json_path", Str, "Location inside the page JSON."),
            opt("source_expr", Str, "Expression text."),
            opt(
                "source_field",
                Str,
                "Property holding the expression (visibleCondition, exp, ...), on component edges.",
            ),
            opt(
                "source_file",
                Str,
                "Page of the referring component, on component -> component edges.",
            ),
            opt("target_component", Str, "Referenced component id."),
            opt(
                "target_property",
                Str,
                "Property of the referenced component, when the reference names one.",
            ),
            opt("target_param", Str, "Referenced parameter name."),
            opt(
                "target_user_property",
                Str,
                "Referenced user property name, on component -> user: edges.",
            ),
            opt(
                "target_system_var",
                Str,
                "Referenced system variable name, on component -> system: edges.",
            ),
            opt(
                "condition_type",
                Str,
                "On condition -> owner edges: the condition kind.",
            ),
        ],
        producers: &[
            producer(
                SPG,
                "A component expression that references another component (value or property), a parameter, a user property or a system variable: Component -> that symbol, operation DependsOn.",
            ),
            producer(
                SPG,
                "Every Condition node: Condition -> its owner (component, action or model source), operation Conditions.",
            ),
            producer(
                SPG,
                "Every symbol a Condition references (component, param, model, user, system): Condition -> that symbol, operation DependsOn.",
            ),
            producer(
                SPG,
                "A model source filter: Condition -> the model's implicit field totalRowCount__, operation DependsOn.",
            ),
        ],
        absent_when: "Only symbols the expression parser recognises (component, param, user, system, model field) produce edges; other text is not tracked. A filter on a dwtable source that nothing else on the page references loses its Condition -> model owner edge: the scanner writes condition edges before it creates that Model node, and the store drops an edge whose endpoint is missing. The Condition node and its DependsOn edge to totalRowCount__ are still there.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DataflowInput,
        summary: "Dataflow lineage: a model takes another model as input (Model -> its input model). Also the local-model-to-table link of a page source.",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the input table as written in the referring metadata.",
        meta_keys: NO_META,
        producers: &[
            producer(
                TBL,
                "A dataflow .tbl: each node's moduleTablePath, and each entry of properties.depends, becomes DataflowInput to that table's model.",
            ),
            producer(
                SPG,
                "A dataflow embedded in a page: each node's moduleTablePath. A dwtable page source: local model -> the physical table it is bound to.",
            ),
        ],
        absent_when: "Model-level only; field-level lineage is not stored (see FieldAlias and the --explain verb).",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DataflowOutput,
        summary: "Reverse of the page-source binding: a physical table points back at the local model bound to it.",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the table as declared by the page source.",
        meta_keys: NO_META,
        producers: &[producer(
            SPG,
            "A dwtable page source: physical table -> local model, written next to the DataflowInput in the other direction so a walk can go from the table back to the page source.",
        )],
        absent_when: "Only dwtable page sources produce it; a dataflow's own output is OutputsTo.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::OutputsTo,
        summary: "A table declares its output table (dbTableName). May point at the same model.",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Name of the output table.",
        meta_keys: NO_META,
        producers: &[producer(
            TBL,
            "A .tbl with a non-empty properties.dbTableName (App or DataFlow): model -> model:<dbTableName>.",
        )],
        absent_when: "A table without a dbTableName has no output table, and so no OutputsTo edge.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::FieldAlias,
        summary: "Local model field to physical table field alias (field:model1.name -> field:table1.name).",
        emitted: true,
        endpoints: &[pair(NodeType::Field, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<local model>.<field> on the alias side.",
        meta_keys: NO_META,
        producers: &[
            producer(
                SPG_READ,
                "A read through a local model whose source maps to a physical table: local field -> physical field, written next to the read edge.",
            ),
            producer(
                SPG_WRITE,
                "The same for writes, validations and loads through a local model.",
            ),
        ],
        absent_when: "Only fields whose local model maps to a resolvable physical table get an alias edge. This is the only stored field-to-field relation; multi-hop field lineage is computed at query time, not stored.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::OpensPage,
        summary: "Declared in the enum, never written by the scanner. Page navigation is stored as ActionNavigates.",
        emitted: false,
        endpoints: &[],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not applicable.",
        meta_keys: NO_META,
        producers: &[],
        absent_when: "Always absent; do not query it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::SetsParam,
        summary: "Declared in the enum, never written by the scanner. Parameter assignment is stored as ActionSetsParam; parameter passing as PassesParam.",
        emitted: false,
        endpoints: &[],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not applicable.",
        meta_keys: NO_META,
        producers: &[],
        absent_when: "Always absent; do not query it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DataflowInternal,
        summary: "Declared in the enum, never written by the scanner. Dataflow internals live in the Model node's meta (nodeFields, internalDeps, ...).",
        emitted: false,
        endpoints: &[],
        field_path: FieldPathUse::Never,
        field_path_meaning: "Not applicable.",
        meta_keys: NO_META,
        producers: &[],
        absent_when: "Always absent; do not query it.",
    },
];

// ---------------------------------------------------------------- 解释规则与方言

const INTERPRETATION_RULES: &[InterpretationRule] = &[
    InterpretationRule {
        id: "static-only",
        rule: "The graph holds static metadata only. Runtime facts (whether a parameter is actually passed, whether a table has rows, what a user sees right now) cannot be read from it. Say 'cannot be determined from the graph' instead of guessing.",
    },
    InterpretationRule {
        id: "hidden-is-configuration",
        rule: "A component with a visibleCondition (or disableCondition) is hidden or disabled by configuration under some state. That is intended behaviour, not a rendering fault; do not report it as a defect.",
    },
    InterpretationRule {
        id: "absent-edge-is-not-absent-relation",
        rule: "A missing edge does not always mean the relation does not exist. Check the edge type's absent_when: unresolved navigation or embed targets and unrecognised expression text produce no edge.",
    },
    InterpretationRule {
        id: "model-path-is-not-a-file",
        rule: "A Model node's path can be a declared reference ($DATA:/dir/x.tbl, ../x.tbl), the .spg path of the page that embeds a dataflow, or a placeholder '<name>.tbl' for an output table or a model that was never resolved. Do not treat a Model path as proof that the file exists.",
    },
    InterpretationRule {
        id: "dangling-page",
        rule: "A Page node with no Contains edge is not necessarily a reference to a missing file: a scanned page with no canvas has none either. A page was scanned only if an IndexState record with key 'file_state:<path>' exists for it. A Page node without that record was created from a link or embed target, and no scanned file backs it.",
    },
    InterpretationRule {
        id: "depends-on-is-overloaded",
        rule: "DependsOn has two meanings, told apart by meta.operation: 'Conditions' means the Condition node belongs to its owner (Condition -> owner); 'DependsOn' means the source depends on the target symbol.",
    },
    InterpretationRule {
        id: "field-write-is-not-always-a-write",
        rule: "FieldWrite edges are written next to Writes, ActionWrites, ActionValidates and ActionLoadsData. Read meta.operation before calling one a write: ActionValidates is a validation, ActionLoadsData (target field:<model>.*) is a load or reset.",
    },
    InterpretationRule {
        id: "field-nodes-include-symbols",
        rule: "Field nodes include page parameters (param:), user properties (user:) and system variables (system:), not only table fields (field:). The id prefix tells them apart and is authoritative; meta.kind is set only on nodes created from an expression or condition, so a parameter created only by a link or setParamValue action has no meta.",
    },
    InterpretationRule {
        id: "field-path-varies",
        rule: "r.field_path does not mean the same thing on every edge type: usually <model>.<field>, but an expression on PassesParam / ActionSetsParam, a page path on ActionNavigates / EmbedsPage, a table path on DataflowInput / DataflowOutput. See each edge type's field_path_meaning.",
    },
    InterpretationRule {
        id: "model-filters-are-conditions",
        rule: "Filters configured on a model are Condition nodes owned by that model (DependsOn edge with meta.operation = Conditions, json_path starting with sources[...].filter). Read their raw_expr for the filter fields and values.",
    },
];

const EXAMPLES: &[QueryExample] = &[
    QueryExample {
        title: "Which model fields do components read",
        gql: "MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path",
    },
    QueryExample {
        title: "Which nodes carry a visibleCondition",
        gql: "MATCH (n:Node) WHERE n.meta CONTAINS 'visibleCondition' RETURN n.id",
    },
    QueryExample {
        title: "Which actions write which fields (filter FieldWrite by operation)",
        gql: "MATCH (a:Node)-[r:FieldWrite]->(f:Node) WHERE r.meta CONTAINS '\"operation\":\"ActionWrites\"' RETURN a.id, f.id",
    },
    QueryExample {
        title: "Which page does each link action open",
        gql: "MATCH (a:Node)-[r:ActionNavigates]->(p:Node) RETURN a.id, p.id",
    },
    QueryExample {
        title: "Which tables does each dataflow read from",
        gql: "MATCH (m:Node)-[r:DataflowInput]->(t:Node) RETURN m.id, t.id, r.field_path",
    },
    QueryExample {
        title: "Which filters are configured on models",
        gql: "MATCH (c:Node) WHERE c.node_type = 'Condition' AND c.id CONTAINS '#filter#' RETURN c.id, c.meta",
    },
];

const DIALECT_NOTES: &[&str] = &[
    "Several edge types are written [:Reads|Triggers] (no colon before the second name).",
    "Use the CONTAINS / STARTS WITH operators; there is no =~ regex and no CONTAINS() function.",
    "Pattern predicates such as NOT (n)--() are not supported.",
    "meta is JSON text, not a map: return it to read it, filter it with CONTAINS; n.meta.exp is a syntax error.",
    "Always write (n:Node); the graph also holds internal IndexState records.",
];

const LABELS: GraphLabels = GraphLabels {
    project_node: "Node",
    node_properties: &["id", "node_type", "path", "name", "meta", "origin_file"],
    edge_properties: &["field_path", "meta", "origin_file"],
    internal_label: "IndexState",
};

const SCHEMA: GraphSchema = GraphSchema {
    contract_version: GRAPH_SCHEMA_CONTRACT_VERSION,
    labels: LABELS,
    node_types: NODE_TYPES,
    edge_types: EDGE_TYPES,
    interpretation_rules: INTERPRETATION_RULES,
    examples: EXAMPLES,
    dialect_notes: DIALECT_NOTES,
};

/// 返回完整契约。
pub fn schema() -> &'static GraphSchema {
    &SCHEMA
}

impl GraphSchema {
    /// 按节点类型取契约行。
    pub fn node_row(&self, node_type: &NodeType) -> Option<&'static NodeTypeSchema> {
        self.node_types
            .iter()
            .find(|row| &row.node_type == node_type)
    }

    /// 按边类型取契约行。
    pub fn edge_row(&self, edge_type: &EdgeType) -> Option<&'static EdgeTypeSchema> {
        self.edge_types
            .iter()
            .find(|row| &row.edge_type == edge_type)
    }
}

impl MetaValueType {
    /// 契约里的类型名（与 JSON 输出同写法）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Array => "array",
            Self::Object => "object",
            Self::Any => "any",
        }
    }

    /// 一个 JSON 值是否符合该类型（`null` 由 `nullable` 单独决定，不在这里判）。
    pub fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Bool => value.is_boolean(),
            Self::Number => value.is_number(),
            Self::Array => value.is_array(),
            Self::Object => value.is_object(),
            Self::Any => true,
        }
    }
}

/// 边 / 节点类型在图里的名字（即枚举变体名）。
pub fn variant_name(value: &impl std::fmt::Debug) -> String {
    format!("{value:?}")
}

/// `--graph-schema` 的 JSON 输出：契约表本身的序列化，没有手写副本。
pub fn to_json() -> Value {
    serde_json::to_value(SCHEMA).unwrap_or_else(|error| {
        // 契约全是静态数据，序列化不会失败；万一失败要让输出自己说明，而不是静默给空对象
        json!({ "error": format!("failed to serialize the graph schema: {error}") })
    })
}

fn push_meta_keys(out: &mut String, keys: &[MetaKeySchema], indent: &str) {
    for meta_key in keys {
        let marker = if meta_key.required {
            ", always present"
        } else {
            ""
        };
        let null_marker = if meta_key.nullable { " or null" } else { "" };
        let _ = writeln!(
            out,
            "{indent}- `{}` ({}{null_marker}{marker}): {}",
            meta_key.name,
            meta_key.value_type.as_str(),
            meta_key.description
        );
    }
}

/// `--graph-schema --human` 与 `docs/reference/graph-schema.md` 共用的 Markdown。
pub fn to_markdown() -> String {
    let labels = &SCHEMA.labels;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Graph schema (contract version {})",
        SCHEMA.contract_version
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Generated from `src/graph_schema.rs` by `metadata-checker --graph-schema --human`. Do not edit by hand."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Elements");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Project nodes have label `{}` with properties {}.",
        labels.project_node,
        quoted_list(labels.node_properties)
    );
    let _ = writeln!(
        out,
        "- Edges are typed by edge-type name and carry {}.",
        quoted_list(labels.edge_properties)
    );
    let _ = writeln!(
        out,
        "- The graph also holds internal `{}` records; always write `(n:{})`.",
        labels.internal_label, labels.project_node
    );
    let _ = writeln!(out, "- `meta` is JSON text, not a map.");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Node types");
    for row in SCHEMA.node_types {
        let _ = writeln!(out);
        let _ = writeln!(out, "### {}", variant_name(&row.node_type));
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", row.summary);
        let _ = writeln!(out);
        let _ = writeln!(out, "- id: {}", row.id_formats.join(" | "));
        let _ = writeln!(out, "- path: {}", row.path_meaning);
        if row.meta_keys.is_empty() {
            let _ = writeln!(out, "- meta: none");
        } else {
            let open = if row.meta_open {
                " (open: other keys can occur)"
            } else {
                ""
            };
            let _ = writeln!(out, "- meta keys{open}:");
            push_meta_keys(&mut out, row.meta_keys, "  ");
        }
        let _ = writeln!(out, "- notes: {}", row.notes);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Edge types");
    for row in SCHEMA.edge_types {
        let _ = writeln!(out);
        let _ = writeln!(out, "### {}", variant_name(&row.edge_type));
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", row.summary);
        let _ = writeln!(out);
        if !row.emitted {
            let _ = writeln!(out, "- emitted by the scanner: no");
            continue;
        }
        let endpoints: Vec<String> = row
            .endpoints
            .iter()
            .map(|p| format!("{} -> {}", variant_name(&p.from), variant_name(&p.to)))
            .collect();
        let _ = writeln!(out, "- endpoints: {}", endpoints.join("; "));
        let _ = writeln!(
            out,
            "- field_path: {} ({})",
            match row.field_path {
                FieldPathUse::Never => "never set",
                FieldPathUse::Always => "always set",
                FieldPathUse::Sometimes => "set on some edges",
            },
            row.field_path_meaning
        );
        if row.meta_keys.is_empty() {
            let _ = writeln!(out, "- meta: none");
        } else {
            let _ = writeln!(out, "- meta keys:");
            push_meta_keys(&mut out, row.meta_keys, "  ");
        }
        let _ = writeln!(out, "- produced by:");
        for source in row.producers {
            let _ = writeln!(out, "  - `{}`: {}", source.path, source.rule);
        }
        let _ = writeln!(out, "- when absent: {}", row.absent_when);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Interpretation rules");
    let _ = writeln!(out);
    for rule in SCHEMA.interpretation_rules {
        let _ = writeln!(out, "- **{}**: {}", rule.id, rule.rule);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Example queries");
    let _ = writeln!(out);
    for example in SCHEMA.examples {
        let _ = writeln!(out, "- {}:", example.title);
        let _ = writeln!(out, "  `{}`", example.gql);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## GQL dialect notes");
    let _ = writeln!(out);
    for note in SCHEMA.dialect_notes {
        let _ = writeln!(out, "- {note}");
    }
    out
}

/// `` `a`, `b`, `c` `` 形式的列表。
fn quoted_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `--gql` 的 long help：紧凑版，完整内容走 `--graph-schema`。
pub fn gql_long_help() -> String {
    let labels = &SCHEMA.labels;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Run a read-only GQL query on a .grafeo graph (--graph-db-path)."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Schema (full version: --graph-schema):");
    let _ = writeln!(
        out,
        "  - Every project node has label {} with properties {}.",
        labels.project_node,
        labels.node_properties.join(", ")
    );
    let node_types: Vec<String> = SCHEMA
        .node_types
        .iter()
        .map(|r| variant_name(&r.node_type))
        .collect();
    let _ = writeln!(out, "    node_type is one of {}.", node_types.join(", "));
    let _ = writeln!(out, "  - id formats:");
    for row in SCHEMA.node_types {
        for format in row.id_formats {
            let _ = writeln!(out, "      {format}");
        }
    }
    let _ = writeln!(
        out,
        "  - Edges are typed by edge-type name and carry {}. List the types with:",
        labels.edge_properties.join(", ")
    );
    let _ = writeln!(out, "      MATCH ()-[r]->() RETURN DISTINCT type(r)");
    let _ = writeln!(out, "  - Edge types the scanner writes:");
    for row in SCHEMA.edge_types.iter().filter(|row| row.emitted) {
        let endpoints: Vec<String> = row
            .endpoints
            .iter()
            .map(|p| format!("{}>{}", variant_name(&p.from), variant_name(&p.to)))
            .collect();
        let _ = writeln!(
            out,
            "      {}  ({})",
            variant_name(&row.edge_type),
            endpoints.join(", ")
        );
    }
    let _ = writeln!(out, "  - meta keys by node_type:");
    for row in SCHEMA
        .node_types
        .iter()
        .filter(|row| !row.meta_keys.is_empty())
    {
        let names: Vec<&str> = row.meta_keys.iter().map(|k| k.name).collect();
        let _ = writeln!(
            out,
            "      {}: {}",
            variant_name(&row.node_type),
            names.join(", ")
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Examples:");
    for example in SCHEMA.examples {
        let _ = writeln!(out, "  {}", example.gql);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Dialect notes:");
    for note in SCHEMA.dialect_notes {
        let _ = writeln!(out, "  - {note}");
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Read the interpretation rules (--graph-schema) before drawing conclusions from the results."
    );
    let _ = write!(
        out,
        "Only MATCH/OPTIONAL/UNWIND/FOR/RETURN statements are accepted; writes, DDL and LOAD are rejected."
    );
    out
}
