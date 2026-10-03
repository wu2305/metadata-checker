# Graph schema (contract version 1)

Generated from `src/graph_schema.rs` by `metadata-checker --graph-schema --human`. Do not edit by hand.

## Elements

- Project nodes have label `Node` with properties `id`, `node_type`, `path`, `name`, `meta`, `origin_file`.
- Edges are typed by edge-type name and carry `field_path`, `meta`, `origin_file`.
- The graph also holds internal `IndexState` records; always write `(n:Node)`.
- `meta` is JSON text, not a map.

## Node types

### Page

A SuperPage file (.spg), or a page that another page refers to.

- id: page:<project-relative path>
- path: The .spg path. A referenced page is created from the link target even when no file exists there, so a Page node is a dangling reference unless a scanned file backs it (see the dangling-page rule); a missing Contains edge alone does not tell, since a scanned page with no canvas has none.
- meta: none
- notes: No meta.

### Component

A component inside a page (including the canvas root and dialogs).

- id: comp:<page path>|<component id>
- path: The page path the component lives in.
- meta keys:
  - `component_type` (string): The component's type string.
  - `json_path` (string): Location of the component inside the page JSON.
  - `parent_id` (string): Id of the enclosing component; absent on the root.
  - `properties` (object): Object with the extracted properties (exp, value, visibleCondition, disableCondition, submitField, ...).
  - `dataSet` (string): Bound data set, on data-bound components.
  - `source` (string): Data source reference, on data-bound components.
  - `data_context_component_id` (string): Component that supplies the data context.
  - `data_context_dataSet` (string): Data set of that data context.
  - `data_context_json_path` (string): JSON path of that data context.
  - `data_context_source` (string): Source of that data context.
- notes: Some components (for example the canvas root, or a dialog that is only the target of a showDialog action) have no meta.

### Model

A data model: a physical table (.tbl), a dataflow, a model referenced by a page, or an output table of a dataflow.

- id: model:<name> | model:<page path>|<local name> (ownership-bound graphs only; see notes)
- path: Depends on where the node came from: the .tbl file path for a scanned table; the .spg path for a dataflow embedded in a page; the path as declared by the referring file for a referenced table (for example $DATA:/dir/x.tbl or ../x.tbl, not resolved to a file); '<name>.tbl' for an output table or an unresolved model. A declared or placeholder path is not evidence that the file exists.
- meta keys:
  - `modelType` (string): Model kind: App or DataFlow (a scanned .tbl), PhysicalTable (output or referenced table), dwtable (a page source bound to a table), DataFlowDependency (listed in a dataflow's depends).
  - `sourcePath` (string): Declared table path.
  - `landed` (bool): Only present, and false, on an un-landed dataflow: a .tbl dataflow with an empty dbTableName, which fetches data on the fly and has no stored table (and no OutputsTo edge).
  - `dimensions` (array): Array of field definitions (name, dbfield, dataType, isDimension, length).
  - `embeddedIn` (string): Page name, on a dataflow embedded in a page.
  - `aliasMap` (object): Dataflow: alias to table mapping.
  - `internalDeps` (object): Dataflow: dependencies between internal nodes.
  - `nodeFields` (object): Dataflow: fields per internal node.
  - `nodeFilters` (object): Dataflow: filters per internal node.
  - `nodeJoinConditions` (object): Dataflow: join conditions per internal node.
  - `nodeTablePaths` (object): Dataflow: table path per internal node.
  - `nodeTypes` (object): Dataflow: type per internal node.
  - `nodeUnionMaps` (object): Dataflow: union mappings per internal node.
- notes: In the .grafeo graph that --gql reads, model names are global: a page source called model1 on two pages collapses into one node model:model1, and a page source named like a table is the same node as the table. Ownership-bound graphs (redb with a project binding) instead give page-local models the id model:<page path>|<local name>; ownership scanning is not wired for Grafeo, so GQL does not see that form today.

### Field

A table field, or a non-table symbol: a page parameter, a user property or a system variable.

- id: field:<model>.<field> | field:<model>.* (stands for all fields of the model; target of loadData-style writes) | field:<page path>|<model>.<field> (ownership-bound graphs only; see the Model notes) | param:<page path>|<param name> | param:<page name>/<param name> (parameter of a referenced page) | user:<namespace>.<name> (the part after $; the namespace is usually user, but =$project.name gives user:project.name) | system:<name>
- path: The path of the model the field belongs to (the .tbl path for field:, or the .spg path for a field of a dataflow embedded in a page), the page path for param:, and 'system' for user: and system:.
- meta keys (open: other keys can occur):
  - `kind` (string): param | user_property | system_var on non-table symbols; implicit on the model's totalRowCount__ field.
  - `name` (string): Display name of a table field.
  - `dataType` (string): Field data type.
  - `dbfield` (string): Physical column name.
  - `isDimension` (bool): Whether the field is a dimension.
  - `length` (number): Declared length.
  - `description` (string): Field description.
  - `inputField` (string): Dataflow: upstream input field.
  - `source_input_field` (string): Dataflow: upstream input field (source form).
  - `originalField` (string): Dataflow: original field behind an alias.
  - `originalNode` (string): Dataflow: node the original field belongs to.
  - `exp` (string): Dataflow: computed-field expression.
  - `source_expr` (string): Dataflow: expression text the field came from.
  - `source_expr_models` (array): Names of the models the expression refers to (strings), when it refers to any.
  - `dimensionPath` (string): Dataflow: dimension path.
  - `businessDesc` (string): Business description written on the field definition.
  - `fieldRole` (string): Role of the field as authored.
  - `hidden` (bool): Field is hidden, as authored.
  - `isPrimaryKey` (bool): Field is a primary key, as authored.
  - `defaultValueExp` (string): Default-value expression, as authored.
  - `aggType` (string): Aggregation type, as authored.
  - `isAggFun` (bool): Whether the field is an aggregate, as authored.
  - `periodType` (string): Period type, as authored.
  - `dateFormat` (string): Date format, as authored.
  - `dateKeyValueFormat` (string): Date key/value format, as authored.
  - `displayFormat` (string): Display format, as authored.
  - `displayDimension` (string): Display dimension, as authored.
  - `textField` (string): Text field, as authored.
  - `decimal` (number): Decimal places, as authored.
  - `precision` (number): Precision, as authored.
  - `geoType` (string): Geo type, as authored.
  - `extractable` (bool): Extractable flag, as authored.
  - `isAutoInc` (bool): Auto-increment flag, as authored.
  - `newborn` (bool): Newborn flag, as authored.
  - `isOriginalFieldInvalid` (bool): Whether the original field is invalid, as authored.
  - `fileModifyTimeField` (string): File modify-time field, as authored.
  - `fieldStorageInfo` (object): Storage details of the field, as authored.
  - `modifiedInfo` (object): Modification details of the field, as authored.
  - `labels` (array): Labels on the field, as authored.
- notes: For a table field, meta is a copy of the field's dimension definition in the .tbl, plus the source_* keys the scanner adds, so keys other than the ones listed can occur: the platform adds dimension properties over time. field:<model>.* stands for 'all fields of the model'. Most table fields have no meta.

### Action

An action configured on a component (click handler and similar).

- id: action:<page path>|<component id>|<action id>
- path: The page path the action lives in.
- meta keys:
  - `triggerType` (string, always present): Event that fires the action, for example click; may be an empty string.
  - `condition` (any or null, always present): Raw condition value as authored, or null (null on every action seen so far).
  - `conditionExp` (string or null, always present): Condition expression, or null.
  - `waitPrev` (string or null, always present): Which earlier action this one waits for: 'nowait' or a '<component>.<action>' reference; null when not set.
- notes: The action's own type (submitData, link, ...) is not stored in meta; it shows in the edges the action has (see each Action* edge type) and in the node name (<actionType>:<action id>).

### Condition

An expression with a semantic role: a component's exp / visibleCondition / disableCondition, an action's condition or conditionExp, or a model filter clause.

- id: cond:<page path>|<component id>#<property>#<n> (component expression; n is the occurrence index) | cond:<page path>|<component id>#<action id>#conditionExp (action conditionExp) | cond:<page path>|<component id>#<action id>#condition (action condition) | cond:<page path>|<source id>#filter#<n>#exp (model filter expression) | cond:<page path>|<source id>#filter#<n>#clause (model filter clause)
- path: The page path the condition lives in.
- meta keys:
  - `condition_type` (string, always present): What kind of expression this is (for example FieldExp, VisibleCondition, SourceFilterExp).
  - `effect_type` (string, always present): What the expression controls (for example Compute, Show, Filter).
  - `subject_type` (string, always present): Type of the owner.
  - `raw_expr` (string, always present): The expression as written.
  - `normalized_expr` (string, always present): The expression after symbol normalisation.
  - `json_path` (string, always present): Location inside the page JSON.
  - `owner_type` (string, always present): Type of the owner.
  - `owner_id` (string, always present): Id of the owner within the page.
  - `referenced_symbols` (array, always present): Array of symbols (model:..., component:..., param:...) the expression references.
- notes: Model filter clauses are Condition nodes owned by the model (json_path starts with sources[...].filter); read raw_expr for the filter fields.

## Edge types

### Contains

Structural containment: page contains component, component contains component, model contains field.

- endpoints: Page -> Component; Component -> Component; Model -> Field
- field_path: never set (Not set.)
- meta: none
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: Second pass over the page's components: page -> every component, and parent component -> nested component (one Component parent each).
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A dataflow embedded in the page: model -> each of its dimension fields; a model with a filter also contains the implicit field totalRowCount__ (the totalRowCount__ edge is missing when the model node does not exist yet at that point, see DependsOn).
  - `scanner/spg.rs::ensure_model_field_with_scope`: Every model.field a page expression touches: model -> field.
  - `scanner/tbl.rs::process_tbl_file_from_string`: A .tbl: model -> each dimension field; the output table (dbTableName) also contains the same dimension fields.
- when absent: Complete for what the scanner parsed.

### Triggers

A component owns an action (the action fires from that component).

- endpoints: Component -> Action
- field_path: never set (Not set.)
- meta: none
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: Actions pass: every action of every component gets component -> action.
- when absent: Complete for what the scanner parsed.

### Reads

A component's expression reads a model field.

- endpoints: Component -> Field; Component -> Model
- field_path: always set (<model>.<field> that is read; just <model> when the expression names the model without a field (a bare ${model1}), which goes with a Model target.)
- meta keys:
  - `actor_kind` (string): component.
  - `actor_id` (string): Component id.
  - `operation` (string): Reads.
  - `reason` (string): Why the edge exists.
  - `json_path` (string): Where in the page JSON the expression is.
  - `source_expr` (string): The expression text.
  - `source_field` (string): The property holding the expression (exp, value, ...).
  - `target_model` (string): Model read.
  - `target_field` (string): Field read.
  - `bare_symbol` (string): Set when the reference had no model prefix and was resolved through the data context.
  - `resolution` (string): How a bare symbol was resolved.
  - `target_model_path` (string): Path of the resolved model.
  - `data_context_component_id` (string): Data-context component used for resolution.
  - `data_context_dataSet` (string): Data set used for resolution.
  - `data_context_json_path` (string): JSON path used for resolution.
  - `data_context_source` (string or null): Source used for resolution.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A component expression (exp, value, visibleCondition, ...) that references model.field, through add_model_read_with_scope.
  - `scanner/spg.rs::add_model_read_with_scope`: Writes the Component -> Model edge and, when the field name is known, the Component -> Field edge; a local model whose source maps to a physical table gets the same pair again on the physical table.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A bare symbol in a component's value that is not a model, component or parameter, inside an inherited data context: resolved against the context's data set.
- when absent: A component whose expressions reference no model or field has no Reads edge. A component bound through a data context may only show a Reads edge with a bare_symbol resolution.

### Writes

A component's submitField binds the component to a model field it writes.

- endpoints: Component -> Model
- field_path: always set (<model>.<field> that is written.)
- meta keys:
  - `actor_kind` (string): component.
  - `actor_id` (string): Id of the component within its page.
  - `operation` (string): The edge type name.
  - `reason` (string): Human-readable sentence describing why the edge exists.
  - `source_expr` (string): Expression text the edge was derived from.
  - `target_model` (string): Model the edge refers to.
  - `target_field` (string): Field the edge refers to.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A component with properties.submitField of the form model.field: Component -> Model through add_model_write_with_scope (which also adds the field-level FieldWrite edge).
- when absent: Only components with a submitField produce it.

### FieldWrite

Field-level edge written next to a model-level one by the same actor: a component's submitField, or an action. Despite the name it is not always a write: meta.operation says which (Writes, ActionWrites, ActionValidates, ActionLoadsData).

- endpoints: Action -> Field; Component -> Field
- field_path: always set (<model>.<field> that is written, validated or loaded; <model>.* for ActionLoadsData.)
- meta keys:
  - `actor_kind` (string): component | action.
  - `actor_id` (string): Id of the actor.
  - `operation` (string): Writes (component submitField), ActionWrites, ActionValidates or ActionLoadsData: the model-level edge type written next to this one. Filter on it before calling the edge a write.
  - `reason` (string): Why the edge exists.
  - `trigger` (string): Action trigger, on action actors.
  - `source_expr` (string): Value expression, when the write has one.
  - `target_model` (string): Model written.
  - `target_field` (string): Field written; absent on ActionLoadsData.
- produced by:
  - `scanner/spg.rs::add_model_write_with_scope`: Called for every model-level Writes / ActionWrites / ActionValidates / ActionLoadsData edge; when the field name is non-empty it adds Actor -> Field with the same meta, on the local model and again on the physical table.
- when absent: A write whose field cannot be resolved has only the model-level edge.

### ActionWrites

An action writes to a model (submitData, insertData, updateData, deleteData, ...).

- endpoints: Action -> Model
- field_path: always set (<model>.<field> that is written.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionWrites.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `source_expr` (string): Value expression.
  - `target_model` (string): Model written.
  - `target_field` (string): Field written.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: submitData: for each component in the submit range that has a submitField, one edge per model.field.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: updateData / insertData / deleteData: one edge per configured field value of the action's data set.
- when absent: Action types that write nothing have no such edge.

### ActionReads

An action reads a model or field (as a query input, a link parameter value or a set-parameter value).

- endpoints: Action -> Field; Action -> Model
- field_path: always set (<model>.<field> that is read; just <model> when the expression names the model without a field (a bare ${model1}), which goes with a Model target.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): Reads.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `source_expr` (string): Expression text.
  - `target_model` (string): Model read.
  - `target_field` (string): Field read.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: An expression-valued field of an insertData / updateData / deleteData action that references model.field.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A link(app) action's data entries: expressions in parameter values.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A setParamValue action's params: expressions in parameter values.
- when absent: Only expressions that reference a model or field produce it.

### ActionNavigates

A link action opens another page.

- endpoints: Action -> Page
- field_path: always set (Project-relative path of the target page.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionNavigates.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `target_model` (string): Target page name.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A link action with targetType app whose path is an index into referenceResources that resolves to a .spg page path: the target Page node is upserted, then Action -> Page.
- when absent: A link whose target cannot be resolved to a page path (index out of range, an absolute path, a URI, an unknown $-prefix, a target that is not .spg) produces no edge and no Page node; the gap is recorded as a SCANNER_UNRESOLVED_REFERENCE occurrence in the IndexState record scanner_entry:<page path> (a link with no path at all is recorded too, as a missing index). A link whose path resolves but whose file does not exist still produces the edge, to a Page node that no scanned file backs (no file_state record). Absence is not proof there is no navigation. The query-time diagnostic UNRESOLVED_PAGE_NAVIGATION only fires for a target node that is missing from the graph, which this scanner never leaves, so it does not cover either case; use the scanner_entry record.

### PassesParam

A link action passes a value into a parameter of the target page.

- endpoints: Action -> Field
- field_path: always set (The value expression being passed (for example =model1.fieldA), not a model.field path.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): PassesParam.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `target_field` (string): Parameter name.
  - `source_expr` (string): Value expression.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A link(app) action with a resolved target page: one Action -> param:<page name>/<param> edge per data entry.
- when absent: Only link actions that carry parameters and whose target resolves produce it.

### ActionSetsParam

A setParamValue action sets a page parameter.

- endpoints: Action -> Field
- field_path: always set (The value expression being assigned (for example =model1.fieldB).)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionSetsParam.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `target_field` (string): Parameter name.
  - `source_expr` (string): Value expression.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A setParamValue action: one Action -> param:<page path>|<param> edge per entry of the action's params.
- when absent: Only setParamValue actions produce it.

### ActionControlsComponent

An action controls a UI component: opens or closes a dialog, shows or hides a component, switches a panel.

- endpoints: Action -> Component
- field_path: never set (Not set.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionControlsComponent.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Says whether it opens, closes, shows, hides or switches.
  - `target_component` (string): Id of the component shown or hidden, on showComponent / hideComponent.
  - `dialog` (string): Dialog id, for showDialog.
  - `panel` (string or null): Panel id, for switchPanel; null when unset.
  - `panelbook` (string): Panel book id, for switchPanel.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: showComponent / hideComponent: one edge per entry of the action's target components.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: showDialog: the dialog component named by the action.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: closeDialog: points at the component the action sits on (the dialog being closed).
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: switchPanel: the action's panel book.
- when absent: Only UI-control action types produce it. A showComponent / hideComponent whose target id is not a component of the page produces no edge (the scanner does not create the target, and the store drops an edge to a missing node), and nothing in the graph marks the gap.

### ActionValidates

A validateData action validates a model field.

- endpoints: Action -> Model
- field_path: always set (<model>.<field> that is validated.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionValidates.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `source_expr` (string): Expression text.
  - `target_model` (string): Model validated.
  - `target_field` (string): Field validated.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: validateData: for each component in the validate range that has a submitField, one edge per model.field (through add_model_write_with_scope, so a FieldWrite edge with operation ActionValidates comes with it).
- when absent: Only validateData actions over components with a submitField produce it.

### ActionLoadsData

A loadData / resetData style action loads or resets a model, or the components it names.

- endpoints: Action -> Model; Action -> Component
- field_path: set on some edges (Action -> Model: <model>.* (the whole model). Action -> Component: not set.)
- meta keys:
  - `actor_kind` (string): action.
  - `actor_id` (string): Action id.
  - `operation` (string): ActionLoadsData.
  - `trigger` (string): Event that fires the action.
  - `reason` (string): Why the edge exists.
  - `target_model` (string): Model loaded, on Action -> Model edges.
  - `target_component` (string): Component loaded or reset, on Action -> Component edges.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: resetData / newData / refreshModels / refreshData / loadData with a data set, or with components whose submitField names a model: Action -> Model with field <model>.* (through add_model_write_with_scope, so a FieldWrite edge to field:<model>.* with operation ActionLoadsData comes with it).
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: The same action types with only submit components that carry no submitField: Action -> each named Component, no field_path.
- when absent: Only load / reset / refresh action types produce it. A submit component id that is not a component of the page produces no Component edge, and nothing in the graph marks the gap.

### EmbedsPage

A component embeds another page (composition, not a user navigation).

- endpoints: Component -> Page
- field_path: always set (Project-relative path of the embedded page.)
- meta: none
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: An embedsuperpage component whose resPath is an index into referenceResources that resolves to a .spg page path: the target Page node is upserted, then Component -> Page.
- when absent: An embed whose target cannot be resolved (or has no resPath) produces no edge; the gap is recorded as a SCANNER_UNRESOLVED_REFERENCE occurrence in the IndexState record scanner_entry:<page path>. One whose path resolves but whose file does not exist produces the edge to a Page node that no scanned file backs (no file_state record).

### DependsOn

Overloaded edge. operation = DependsOn: an expression depends on an upstream symbol (component value or property, parameter, user property, system variable, model field). operation = Conditions: a Condition node points at the owner it belongs to (Condition -> owner).

- endpoints: Component -> Component; Component -> Field; Condition -> Action; Condition -> Component; Condition -> Field; Condition -> Model
- field_path: always set (For operation = DependsOn: the referenced symbol (for example comp:input2.value, param:param1, $user.id). For operation = Conditions: the json_path of the condition.)
- meta keys:
  - `actor_kind` (string): component | condition.
  - `actor_id` (string): Component id or condition id.
  - `operation` (string): DependsOn or Conditions.
  - `reason` (string): Says whether the condition belongs to its owner or depends on a symbol.
  - `json_path` (string): Location inside the page JSON.
  - `source_expr` (string): Expression text.
  - `source_field` (string): Property holding the expression (visibleCondition, exp, ...), on component edges.
  - `source_file` (string): Page of the referring component, on component -> component edges.
  - `target_component` (string): Referenced component id.
  - `target_property` (string): Property of the referenced component, when the reference names one.
  - `target_param` (string): Referenced parameter name.
  - `target_user_property` (string): Referenced user property name, on component -> user: edges.
  - `target_system_var` (string): Referenced system variable name, on component -> system: edges.
  - `condition_type` (string): On condition -> owner edges: the condition kind.
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A component expression that references another component (value or property), a parameter, a user property or a system variable: Component -> that symbol, operation DependsOn.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: Every Condition node: Condition -> its owner (component, action or model source), operation Conditions.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: Every symbol a Condition references (component, param, model, user, system): Condition -> that symbol, operation DependsOn.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A model source filter: Condition -> the model's implicit field totalRowCount__, operation DependsOn.
- when absent: Only symbols the expression parser recognises (component, param, user, system, model field) produce edges; other text is not tracked. A filter on a dwtable source that nothing else on the page references loses its Condition -> model owner edge: the scanner writes condition edges before it creates that Model node, and the store drops an edge whose endpoint is missing. The Condition node and its DependsOn edge to totalRowCount__ are still there.

### DataflowInput

Dataflow lineage: a model takes another model as input (Model -> its input model). Also the local-model-to-table link of a page source.

- endpoints: Model -> Model
- field_path: always set (Path of the input table as written in the referring metadata.)
- meta: none
- produced by:
  - `scanner/tbl.rs::process_tbl_file_from_string`: A dataflow .tbl: each node's moduleTablePath, and each entry of properties.depends, becomes DataflowInput to that table's model.
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A dataflow embedded in a page: each node's moduleTablePath. A dwtable page source: local model -> the physical table it is bound to.
- when absent: Model-level only; field-level lineage is not stored (see FieldAlias and the --explain verb).

### DataflowOutput

Reverse of the page-source binding: a physical table points back at the local model bound to it.

- endpoints: Model -> Model
- field_path: always set (Path of the table as declared by the page source.)
- meta: none
- produced by:
  - `scanner/spg.rs::process_spg_file_from_value_with_identity`: A dwtable page source: physical table -> local model, written next to the DataflowInput in the other direction so a walk can go from the table back to the page source.
- when absent: Only dwtable page sources produce it; a dataflow's own output is OutputsTo.

### OutputsTo

A table declares its output table (dbTableName). May point at the same model.

- endpoints: Model -> Model
- field_path: always set (Name of the output table.)
- meta: none
- produced by:
  - `scanner/tbl.rs::process_tbl_file_from_string`: A .tbl with a non-empty properties.dbTableName (App or DataFlow): model -> model:<dbTableName>.
- when absent: A table without a dbTableName has no output table, and so no OutputsTo edge.

### FieldAlias

Local model field to physical table field alias (field:model1.name -> field:table1.name).

- endpoints: Field -> Field
- field_path: always set (<local model>.<field> on the alias side.)
- meta: none
- produced by:
  - `scanner/spg.rs::add_model_read_with_scope`: A read through a local model whose source maps to a physical table: local field -> physical field, written next to the read edge.
  - `scanner/spg.rs::add_model_write_with_scope`: The same for writes, validations and loads through a local model.
- when absent: Only fields whose local model maps to a resolvable physical table get an alias edge. This is the only stored field-to-field relation; multi-hop field lineage is computed at query time, not stored.

### OpensPage

Declared in the enum, never written by the scanner. Page navigation is stored as ActionNavigates.

- emitted by the scanner: no

### SetsParam

Declared in the enum, never written by the scanner. Parameter assignment is stored as ActionSetsParam; parameter passing as PassesParam.

- emitted by the scanner: no

### DataflowInternal

Declared in the enum, never written by the scanner. Dataflow internals live in the Model node's meta (nodeFields, internalDeps, ...).

- emitted by the scanner: no

## Interpretation rules

- **static-only**: The graph holds static metadata only. Runtime facts (whether a parameter is actually passed, whether a table has rows, what a user sees right now) cannot be read from it. Say 'cannot be determined from the graph' instead of guessing.
- **hidden-is-configuration**: A component with a visibleCondition (or disableCondition) is hidden or disabled by configuration under some state. That is intended behaviour, not a rendering fault; do not report it as a defect.
- **absent-edge-is-not-absent-relation**: A missing edge does not always mean the relation does not exist. Check the edge type's absent_when: unresolved navigation or embed targets (recorded in the page's scanner_entry record) and unrecognised expression text produce no edge.
- **model-path-is-not-a-file**: A Model node's path can be a declared reference ($DATA:/dir/x.tbl, ../x.tbl), the .spg path of the page that embeds a dataflow, or a placeholder '<name>.tbl' for an output table or a model that was never resolved. Do not treat a Model path as proof that the file exists.
- **dangling-page**: A Page node with no Contains edge is not necessarily a reference to a missing file: a scanned page with no canvas has none either. A page was scanned only if an IndexState record with key 'file_state:<path>' exists for it. A Page node without that record was created from a link or embed target, and no scanned file backs it.
- **depends-on-is-overloaded**: DependsOn has two meanings, told apart by meta.operation: 'Conditions' means the Condition node belongs to its owner (Condition -> owner); 'DependsOn' means the source depends on the target symbol.
- **field-write-is-not-always-a-write**: FieldWrite edges are written next to Writes, ActionWrites, ActionValidates and ActionLoadsData. Read meta.operation before calling one a write: ActionValidates is a validation, ActionLoadsData (target field:<model>.*) is a load or reset.
- **field-nodes-include-symbols**: Field nodes include page parameters (param:), user properties (user:) and system variables (system:), not only table fields (field:). The id prefix tells them apart and is authoritative; meta.kind is set only on nodes created from an expression or condition, so a parameter created only by a link or setParamValue action has no meta.
- **field-path-varies**: r.field_path does not mean the same thing on every edge type: usually <model>.<field>, but an expression on PassesParam / ActionSetsParam, a page path on ActionNavigates / EmbedsPage, a table path on DataflowInput / DataflowOutput. See each edge type's field_path_meaning.
- **model-filters-are-conditions**: Filters configured on a model are Condition nodes owned by that model (DependsOn edge with meta.operation = Conditions, json_path starting with sources[...].filter). Read their raw_expr for the filter fields and values.

## Example queries

- Which model fields do components read:
  `MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path`
- Which nodes carry a visibleCondition:
  `MATCH (n:Node) WHERE n.meta CONTAINS 'visibleCondition' RETURN n.id`
- Which actions write which fields (filter FieldWrite by operation):
  `MATCH (a:Node)-[r:FieldWrite]->(f:Node) WHERE r.meta CONTAINS '"operation":"ActionWrites"' RETURN a.id, f.id`
- Which page does each link action open:
  `MATCH (a:Node)-[r:ActionNavigates]->(p:Node) RETURN a.id, p.id`
- Which tables does each dataflow read from:
  `MATCH (m:Node)-[r:DataflowInput]->(t:Node) RETURN m.id, t.id, r.field_path`
- Which filters are configured on models:
  `MATCH (c:Node) WHERE c.node_type = 'Condition' AND c.id CONTAINS '#filter#' RETURN c.id, c.meta`

## GQL dialect notes

- Several edge types are written [:Reads|Triggers] (no colon before the second name).
- Use the CONTAINS / STARTS WITH operators; there is no =~ regex and no CONTAINS() function.
- Pattern predicates such as NOT (n)--() are not supported.
- meta is JSON text, not a map: return it to read it, filter it with CONTAINS; n.meta.exp is a syntax error.
- Always write (n:Node); the graph also holds internal IndexState records.
