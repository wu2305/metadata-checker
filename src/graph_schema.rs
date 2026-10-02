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
//! 与存储层版本无关：[`GRAPH_SCHEMA_CONTRACT_VERSION`] 描述本契约，不是 redb 的
//! `fact_schema_version`；S3 引入图内新事实时才会递增。

use crate::graph::{EdgeType, NodeType};
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// 契约版本。S3（字段级血缘边）与 S4（诊断入图）会让它变成 2。
pub const GRAPH_SCHEMA_CONTRACT_VERSION: u32 = 1;

/// `meta` 里的一个键。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MetaKeySchema {
    pub name: &'static str,
    /// 写入点是否无条件写出该键。
    pub required: bool,
    pub description: &'static str,
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
    pub notes: &'static str,
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
    /// 这种边缺席时，「没有该关系」是否成立；不成立时写明原因。
    pub absent_when: &'static str,
}

/// 解释规则：由图里的事实能下什么结论、不能下什么。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct InterpretationRule {
    pub id: &'static str,
    pub rule: &'static str,
}

/// 完整契约。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GraphSchema {
    pub contract_version: u32,
    pub node_types: &'static [NodeTypeSchema],
    pub edge_types: &'static [EdgeTypeSchema],
    pub interpretation_rules: &'static [InterpretationRule],
    pub dialect_notes: &'static [&'static str],
}

const fn key(name: &'static str, required: bool, description: &'static str) -> MetaKeySchema {
    MetaKeySchema {
        name,
        required,
        description,
    }
}

const fn pair(from: NodeType, to: NodeType) -> EndpointPair {
    EndpointPair { from, to }
}

// ---------------------------------------------------------------- 节点

const NODE_TYPES: &[NodeTypeSchema] = &[
    NodeTypeSchema {
        node_type: NodeType::Page,
        summary: "A SuperPage file (.spg).",
        id_prefixes: &["page"],
        id_formats: &["page:<project-relative path>"],
        path_meaning: "The .spg path (a referenced page may carry an unresolved relative path such as app/../dir/x.spg).",
        meta_keys: &[],
        notes: "No meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Component,
        summary: "A component inside a page (including the canvas root and dialogs).",
        id_prefixes: &["comp"],
        id_formats: &["comp:<page path>|<component id>"],
        path_meaning: "The page path the component lives in.",
        meta_keys: &[
            key("component_type", false, "The component's type string."),
            key(
                "json_path",
                false,
                "Location of the component inside the page JSON.",
            ),
            key(
                "parent_id",
                false,
                "Id of the enclosing component; absent on the root.",
            ),
            key(
                "properties",
                false,
                "Object with the extracted properties (exp, value, visibleCondition, disableCondition, submitField, ...).",
            ),
            key(
                "dataSet",
                false,
                "Bound data set, on data-bound components.",
            ),
            key(
                "source",
                false,
                "Data source reference, on data-bound components.",
            ),
            key(
                "data_context_component_id",
                false,
                "Component that supplies the data context.",
            ),
            key(
                "data_context_dataSet",
                false,
                "Data set of that data context.",
            ),
            key(
                "data_context_json_path",
                false,
                "JSON path of that data context.",
            ),
            key("data_context_source", false, "Source of that data context."),
        ],
        notes: "Some components (for example the canvas root) have no meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Model,
        summary: "A data model: a physical table (.tbl), a dataflow node, or a model referenced by a page.",
        id_prefixes: &["model"],
        id_formats: &["model:<name>"],
        path_meaning: "The .tbl path. When the model could not be resolved to a file, path is a placeholder '<name>.tbl' and is not evidence that such a file exists.",
        meta_keys: &[
            key(
                "modelType",
                false,
                "Model kind, for example dwtable, App, DataFlow.",
            ),
            key("sourcePath", false, "Declared table path."),
            key(
                "dimensions",
                false,
                "Array of field definitions (name, dbfield, dataType, isDimension, length).",
            ),
            key(
                "embeddedIn",
                false,
                "Page name, on a dataflow embedded in a page.",
            ),
            key("aliasMap", false, "Dataflow: alias to table mapping."),
            key(
                "internalDeps",
                false,
                "Dataflow: dependencies between internal nodes.",
            ),
            key("nodeFields", false, "Dataflow: fields per internal node."),
            key("nodeFilters", false, "Dataflow: filters per internal node."),
            key(
                "nodeJoinConditions",
                false,
                "Dataflow: join conditions per internal node.",
            ),
            key(
                "nodeTablePaths",
                false,
                "Dataflow: table path per internal node.",
            ),
            key("nodeTypes", false, "Dataflow: type per internal node."),
            key(
                "nodeUnionMaps",
                false,
                "Dataflow: union mappings per internal node.",
            ),
        ],
        notes: "Several pages may mention the same model name; they share one node (model:<name>).",
    },
    NodeTypeSchema {
        node_type: NodeType::Field,
        summary: "A table field, or a non-table symbol: a page parameter, a user property or a system variable.",
        id_prefixes: &["field", "param", "user", "system"],
        id_formats: &[
            "field:<model>.<field>",
            "param:<page path>|<param name>",
            "param:<page name>/<param name> (parameter of a referenced page)",
            "user:user.<NAME>",
            "system:<name>",
        ],
        path_meaning: "The .tbl path for field:, the page path for param:, and 'system' for user: and system:.",
        meta_keys: &[
            key(
                "kind",
                false,
                "param | user_property | system_var on non-table symbols.",
            ),
            key("name", false, "Display name of a table field."),
            key("dataType", false, "Field data type."),
            key("dbfield", false, "Physical column name."),
            key("isDimension", false, "Whether the field is a dimension."),
            key("length", false, "Declared length."),
            key("description", false, "Field description."),
            key("inputField", false, "Dataflow: upstream input field."),
            key(
                "source_input_field",
                false,
                "Dataflow: upstream input field (source form).",
            ),
            key(
                "originalField",
                false,
                "Dataflow: original field behind an alias.",
            ),
            key(
                "originalNode",
                false,
                "Dataflow: node the original field belongs to.",
            ),
            key("exp", false, "Dataflow: computed-field expression."),
            key(
                "source_expr",
                false,
                "Dataflow: expression text the field came from.",
            ),
            key("dimensionPath", false, "Dataflow: dimension path."),
        ],
        notes: "field:<model>.* stands for 'all fields of the model'. Most table fields have no meta.",
    },
    NodeTypeSchema {
        node_type: NodeType::Action,
        summary: "An action configured on a component (click handler and similar).",
        id_prefixes: &["action"],
        id_formats: &["action:<page path>|<component id>|<action id>"],
        path_meaning: "The page path the action lives in.",
        meta_keys: &[
            key(
                "triggerType",
                true,
                "Event that fires the action, for example click.",
            ),
            key("condition", true, "Raw condition value, or null."),
            key("conditionExp", true, "Condition expression, or null."),
            key(
                "waitPrev",
                true,
                "Whether the action waits for the previous one, or null.",
            ),
        ],
        notes: "The action's own type (submitData, link, ...) is not stored in meta; it shows in the edges the action has (see each Action* edge type) and in the node name.",
    },
    NodeTypeSchema {
        node_type: NodeType::Condition,
        summary: "An expression with a semantic role: a component's exp / visibleCondition / disableCondition, an action's conditionExp, or a model filter clause.",
        id_prefixes: &["cond"],
        id_formats: &[
            "cond:<page path>|<owner id>#<field>#<n>",
            "cond:<page path>|<component>#<action>#conditionExp",
        ],
        path_meaning: "The page path the condition lives in.",
        meta_keys: &[
            key(
                "condition_type",
                true,
                "What kind of expression this is (for example FieldExp, Visible).",
            ),
            key(
                "effect_type",
                true,
                "What the expression controls (for example Compute).",
            ),
            key("subject_type", true, "Type of the owner."),
            key("raw_expr", true, "The expression as written."),
            key(
                "normalized_expr",
                true,
                "The expression after symbol normalisation.",
            ),
            key("json_path", true, "Location inside the page JSON."),
            key("owner_type", true, "Type of the owner."),
            key("owner_id", true, "Id of the owner within the page."),
            key(
                "referenced_symbols",
                true,
                "Array of symbols (model:..., component:..., param:...) the expression references.",
            ),
        ],
        notes: "Model filter clauses are Condition nodes owned by the model (json_path starts with sources[...].filter); read raw_expr for the filter fields.",
    },
];

// ---------------------------------------------------------------- 边

/// 由组件 / action 产生的边共有的来源键。
const ACTOR_KEYS_COMPONENT: &[MetaKeySchema] = &[
    key("actor_kind", false, "component | action | condition."),
    key("actor_id", false, "Id of the actor within its page."),
    key(
        "operation",
        false,
        "The edge type name, or Conditions for a condition-to-owner edge.",
    ),
    key(
        "reason",
        false,
        "Human-readable sentence describing why the edge exists.",
    ),
    key(
        "source_expr",
        false,
        "Expression text the edge was derived from.",
    ),
    key("target_model", false, "Model the edge refers to."),
    key("target_field", false, "Field the edge refers to."),
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
        field_path_meaning: "<model>.<field> that is read.",
        meta_keys: &[
            key("actor_kind", false, "component."),
            key("actor_id", false, "Component id."),
            key("operation", false, "Reads."),
            key("reason", false, "Why the edge exists."),
            key(
                "json_path",
                false,
                "Where in the page JSON the expression is.",
            ),
            key("source_expr", false, "The expression text."),
            key(
                "source_field",
                false,
                "The property holding the expression (exp, value, ...).",
            ),
            key("target_model", false, "Model read."),
            key("target_field", false, "Field read."),
            key(
                "bare_symbol",
                false,
                "Set when the reference had no model prefix and was resolved through the data context.",
            ),
            key("resolution", false, "How a bare symbol was resolved."),
            key("target_model_path", false, "Path of the resolved model."),
            key(
                "data_context_component_id",
                false,
                "Data-context component used for resolution.",
            ),
            key(
                "data_context_dataSet",
                false,
                "Data set used for resolution.",
            ),
            key(
                "data_context_json_path",
                false,
                "JSON path used for resolution.",
            ),
            key("data_context_source", false, "Source used for resolution."),
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
        absent_when: "Only components with a submitField produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::FieldWrite,
        summary: "Field-level write: a component (via submitField) or an action writes a specific field. Usually accompanies Writes / ActionWrites, which point at the model.",
        emitted: true,
        endpoints: &[
            pair(NodeType::Action, NodeType::Field),
            pair(NodeType::Component, NodeType::Field),
        ],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is written.",
        meta_keys: &[
            key("actor_kind", false, "component | action."),
            key("actor_id", false, "Id of the writer."),
            key("operation", false, "Writes."),
            key("reason", false, "Why the edge exists."),
            key("trigger", false, "Action trigger, on action writers."),
            key(
                "source_expr",
                false,
                "Value expression, when the write has one.",
            ),
            key("target_model", false, "Model written."),
            key("target_field", false, "Field written, when known."),
        ],
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
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionWrites."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("source_expr", false, "Value expression."),
            key("target_model", false, "Model written."),
            key("target_field", false, "Field written."),
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
        field_path_meaning: "<model>.<field> that is read.",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "Reads."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("source_expr", false, "Expression text."),
            key("target_model", false, "Model read."),
            key("target_field", false, "Field read."),
        ],
        absent_when: "Only expressions that reference a model or field produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionNavigates,
        summary: "A link action opens another page.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Page)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the target page as written (may contain ../).",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionNavigates."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("target_model", false, "Target page name."),
        ],
        absent_when: "A link whose target page cannot be resolved produces no edge here; the query layer reports it as UNRESOLVED_PAGE_NAVIGATION. Absence is not proof there is no navigation.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::PassesParam,
        summary: "A link action passes a value into a parameter of the target page.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "The value expression being passed (for example =model1.fieldA), not a model.field path.",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "PassesParam."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("target_field", false, "Parameter name."),
            key("source_expr", false, "Value expression."),
        ],
        absent_when: "Only link actions that carry parameters produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionSetsParam,
        summary: "A setParamValue action sets a page parameter.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "The value expression being assigned (for example =model1.fieldB).",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionSetsParam."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("target_field", false, "Parameter name."),
            key("source_expr", false, "Value expression."),
        ],
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
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionControlsComponent."),
            key("trigger", false, "Event that fires the action."),
            key(
                "reason",
                false,
                "Says whether it opens, closes, shows, hides or switches.",
            ),
            key("dialog", false, "Dialog id, for dialog actions."),
            key("panel", false, "Panel id, for panel switches."),
            key("panelbook", false, "Panel book id, for panel switches."),
        ],
        absent_when: "Only UI-control action types produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionValidates,
        summary: "A validateData action validates a model field.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.<field> that is validated.",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionValidates."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("source_expr", false, "Expression text."),
            key("target_model", false, "Model validated."),
            key("target_field", false, "Field validated."),
        ],
        absent_when: "Only validateData actions produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::ActionLoadsData,
        summary: "A loadData / resetData style action loads or resets a model.",
        emitted: true,
        endpoints: &[pair(NodeType::Action, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<model>.* (the whole model).",
        meta_keys: &[
            key("actor_kind", false, "action."),
            key("actor_id", false, "Action id."),
            key("operation", false, "ActionLoadsData."),
            key("trigger", false, "Event that fires the action."),
            key("reason", false, "Why the edge exists."),
            key("target_model", false, "Model loaded."),
        ],
        absent_when: "Only load / reset / refresh action types produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::EmbedsPage,
        summary: "A component embeds another page (composition, not a user navigation).",
        emitted: true,
        endpoints: &[pair(NodeType::Component, NodeType::Page)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the embedded page as written (may contain ../).",
        meta_keys: NO_META,
        absent_when: "An embed whose target cannot be resolved produces no edge.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DependsOn,
        summary: "Overloaded edge. operation = DependsOn: an expression depends on an upstream symbol (component value, parameter, user property, system variable, model field). operation = Conditions: a Condition node points at the owner it belongs to (Condition -> owner).",
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
        field_path_meaning: "For operation = DependsOn: the referenced symbol (for example comp:input2.value, param:param1). For operation = Conditions: the json_path of the condition.",
        meta_keys: &[
            key("actor_kind", false, "component | condition."),
            key("actor_id", false, "Component id or condition id."),
            key("operation", false, "DependsOn or Conditions."),
            key(
                "reason",
                false,
                "Says whether the condition belongs to its owner or depends on a symbol.",
            ),
            key("json_path", false, "Location inside the page JSON."),
            key("source_expr", false, "Expression text."),
            key(
                "source_field",
                false,
                "Property holding the expression (visibleCondition, exp, ...).",
            ),
            key(
                "source_file",
                false,
                "File of the referenced symbol, for cross-file references.",
            ),
            key("target_component", false, "Referenced component id."),
            key("target_param", false, "Referenced parameter name."),
            key(
                "condition_type",
                false,
                "On condition edges: the condition kind.",
            ),
        ],
        absent_when: "Only symbols the expression parser recognises (component, param, user, system, model field) produce edges; other text is not tracked.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DataflowInput,
        summary: "Dataflow lineage: a model takes another model as input (Model -> its input model).",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the input table.",
        meta_keys: NO_META,
        absent_when: "Model-level only; field-level lineage is not stored (see FieldAlias and the --explain verb).",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::DataflowOutput,
        summary: "Dataflow lineage: a model outputs to another model (Model -> its output model).",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Path of the output table.",
        meta_keys: NO_META,
        absent_when: "Model-level only; field-level lineage is not stored.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::OutputsTo,
        summary: "A dataflow table declares its output target. May point at the same model.",
        emitted: true,
        endpoints: &[pair(NodeType::Model, NodeType::Model)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "Name of the output target.",
        meta_keys: NO_META,
        absent_when: "Only dataflow tables with a declared output produce it.",
    },
    EdgeTypeSchema {
        edge_type: EdgeType::FieldAlias,
        summary: "Local model field to physical table field alias (field:model1.name -> field:table1.name).",
        emitted: true,
        endpoints: &[pair(NodeType::Field, NodeType::Field)],
        field_path: FieldPathUse::Always,
        field_path_meaning: "<local model>.<field> on the alias side.",
        meta_keys: NO_META,
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
        rule: "A missing edge does not always mean the relation does not exist. Check the edge type's absent_when: unresolved navigation targets and unrecognised expression text produce no edge.",
    },
    InterpretationRule {
        id: "placeholder-model-path",
        rule: "When a model was referenced but never resolved to a file, its node exists with a placeholder path '<name>.tbl'. Do not treat that path as a real file.",
    },
    InterpretationRule {
        id: "depends-on-is-overloaded",
        rule: "DependsOn has two meanings, told apart by meta.operation: 'Conditions' means the Condition node belongs to its owner (Condition -> owner); 'DependsOn' means the source depends on the target symbol.",
    },
    InterpretationRule {
        id: "field-nodes-include-symbols",
        rule: "Field nodes include page parameters (param:), user properties (user:) and system variables (system:), not only table fields (field:). meta.kind tells them apart.",
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

const DIALECT_NOTES: &[&str] = &[
    "Several edge types are written [:Reads|Triggers] (no colon before the second name).",
    "Use the CONTAINS / STARTS WITH operators; there is no =~ regex and no CONTAINS() function.",
    "Pattern predicates such as NOT (n)--() are not supported.",
    "meta is JSON text, not a map: return it to read it, filter it with CONTAINS; n.meta.exp is a syntax error.",
    "Always write (n:Node); the graph also holds internal IndexState records.",
];

const SCHEMA: GraphSchema = GraphSchema {
    contract_version: GRAPH_SCHEMA_CONTRACT_VERSION,
    node_types: NODE_TYPES,
    edge_types: EDGE_TYPES,
    interpretation_rules: INTERPRETATION_RULES,
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

/// 边 / 节点类型在图里的名字（即枚举变体名）。
pub fn variant_name(value: &impl std::fmt::Debug) -> String {
    format!("{value:?}")
}

/// `--graph-schema` 的 JSON 输出。
pub fn to_json() -> Value {
    json!({
        "contract_version": SCHEMA.contract_version,
        "labels": {
            "project_node": "Node",
            "node_properties": ["id", "node_type", "path", "name", "meta", "origin_file"],
            "edge_properties": ["field_path", "meta", "origin_file"],
            "internal_label": "IndexState"
        },
        "node_types": SCHEMA.node_types,
        "edge_types": SCHEMA.edge_types,
        "interpretation_rules": SCHEMA.interpretation_rules,
        "dialect_notes": SCHEMA.dialect_notes,
    })
}

fn push_meta_keys(out: &mut String, keys: &[MetaKeySchema], indent: &str) {
    for meta_key in keys {
        let marker = if meta_key.required {
            " (always present)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{indent}- `{}`{marker}: {}",
            meta_key.name, meta_key.description
        );
    }
}

/// `--graph-schema --human` 与 `docs/reference/graph-schema.md` 共用的 Markdown。
pub fn to_markdown() -> String {
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
        "- Project nodes have label `Node` with properties `id`, `node_type`, `path`, `name`, `meta`, `origin_file`."
    );
    let _ = writeln!(
        out,
        "- Edges are typed by edge-type name and carry `field_path`, `meta`, `origin_file`."
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
            let _ = writeln!(out, "- meta keys:");
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
            },
            row.field_path_meaning
        );
        if row.meta_keys.is_empty() {
            let _ = writeln!(out, "- meta: none");
        } else {
            let _ = writeln!(out, "- meta keys:");
            push_meta_keys(&mut out, row.meta_keys, "  ");
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
    let _ = writeln!(out, "## GQL dialect notes");
    let _ = writeln!(out);
    for note in SCHEMA.dialect_notes {
        let _ = writeln!(out, "- {note}");
    }
    out
}

/// `--gql` 的 long help：紧凑版，完整内容走 `--graph-schema`。
pub fn gql_long_help() -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Run a read-only GQL query on a .grafeo graph (--graph-db-path)."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Schema (full version: --graph-schema):");
    let _ = writeln!(
        out,
        "  - Every project node has label Node with properties id, node_type, path, name, meta, origin_file."
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
        "  - Edges are typed by edge-type name and carry field_path, meta, origin_file. List the types with:"
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
    let _ = writeln!(
        out,
        "  MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path"
    );
    let _ = writeln!(
        out,
        "  MATCH (n:Node) WHERE n.meta CONTAINS 'visibleCondition' RETURN n.id"
    );
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
