# Graph schema (contract version 1)

Generated from `src/graph_schema.rs` by `metadata-checker --graph-schema --human`. Do not edit by hand.

## Elements

- Project nodes have label `Node` with properties `id`, `node_type`, `path`, `name`, `meta`, `origin_file`.
- Edges are typed by edge-type name and carry `field_path`, `meta`, `origin_file`.
- `meta` is JSON text, not a map.

## Node types

### Page

A SuperPage file (.spg).

- id: page:<project-relative path>
- path: The .spg path (a referenced page may carry an unresolved relative path such as app/../dir/x.spg).
- meta: none
- notes: No meta.

### Component

A component inside a page (including the canvas root and dialogs).

- id: comp:<page path>|<component id>
- path: The page path the component lives in.
- meta keys:
  - `component_type`: The component's type string.
  - `json_path`: Location of the component inside the page JSON.
  - `parent_id`: Id of the enclosing component; absent on the root.
  - `properties`: Object with the extracted properties (exp, value, visibleCondition, disableCondition, submitField, ...).
  - `dataSet`: Bound data set, on data-bound components.
  - `source`: Data source reference, on data-bound components.
  - `data_context_component_id`: Component that supplies the data context.
  - `data_context_dataSet`: Data set of that data context.
  - `data_context_json_path`: JSON path of that data context.
  - `data_context_source`: Source of that data context.
- notes: Some components (for example the canvas root) have no meta.

### Model

A data model: a physical table (.tbl), a dataflow node, or a model referenced by a page.

- id: model:<name>
- path: The .tbl path. When the model could not be resolved to a file, path is a placeholder '<name>.tbl' and is not evidence that such a file exists.
- meta keys:
  - `modelType`: Model kind, for example dwtable, App, DataFlow.
  - `sourcePath`: Declared table path.
  - `dimensions`: Array of field definitions (name, dbfield, dataType, isDimension, length).
  - `embeddedIn`: Page name, on a dataflow embedded in a page.
  - `aliasMap`: Dataflow: alias to table mapping.
  - `internalDeps`: Dataflow: dependencies between internal nodes.
  - `nodeFields`: Dataflow: fields per internal node.
  - `nodeFilters`: Dataflow: filters per internal node.
  - `nodeJoinConditions`: Dataflow: join conditions per internal node.
  - `nodeTablePaths`: Dataflow: table path per internal node.
  - `nodeTypes`: Dataflow: type per internal node.
  - `nodeUnionMaps`: Dataflow: union mappings per internal node.
- notes: Several pages may mention the same model name; they share one node (model:<name>).

### Field

A table field, or a non-table symbol: a page parameter, a user property or a system variable.

- id: field:<model>.<field> | param:<page path>|<param name> | param:<page name>/<param name> (parameter of a referenced page) | user:user.<NAME> | system:<name>
- path: The .tbl path for field:, the page path for param:, and 'system' for user: and system:.
- meta keys:
  - `kind`: param | user_property | system_var on non-table symbols.
  - `name`: Display name of a table field.
  - `dataType`: Field data type.
  - `dbfield`: Physical column name.
  - `isDimension`: Whether the field is a dimension.
  - `length`: Declared length.
  - `description`: Field description.
  - `inputField`: Dataflow: upstream input field.
  - `source_input_field`: Dataflow: upstream input field (source form).
  - `originalField`: Dataflow: original field behind an alias.
  - `originalNode`: Dataflow: node the original field belongs to.
  - `exp`: Dataflow: computed-field expression.
  - `source_expr`: Dataflow: expression text the field came from.
  - `dimensionPath`: Dataflow: dimension path.
- notes: field:<model>.* stands for 'all fields of the model'. Most table fields have no meta.

### Action

An action configured on a component (click handler and similar).

- id: action:<page path>|<component id>|<action id>
- path: The page path the action lives in.
- meta keys:
  - `triggerType` (always present): Event that fires the action, for example click.
  - `condition` (always present): Raw condition value, or null.
  - `conditionExp` (always present): Condition expression, or null.
  - `waitPrev` (always present): Whether the action waits for the previous one, or null.
- notes: The action's own type (submitData, link, ...) is not stored in meta; it shows in the edges the action has (see each Action* edge type) and in the node name.

### Condition

An expression with a semantic role: a component's exp / visibleCondition / disableCondition, an action's conditionExp, or a model filter clause.

- id: cond:<page path>|<owner id>#<field>#<n> | cond:<page path>|<component>#<action>#conditionExp
- path: The page path the condition lives in.
- meta keys:
  - `condition_type` (always present): What kind of expression this is (for example FieldExp, Visible).
  - `effect_type` (always present): What the expression controls (for example Compute).
  - `subject_type` (always present): Type of the owner.
  - `raw_expr` (always present): The expression as written.
  - `normalized_expr` (always present): The expression after symbol normalisation.
  - `json_path` (always present): Location inside the page JSON.
  - `owner_type` (always present): Type of the owner.
  - `owner_id` (always present): Id of the owner within the page.
  - `referenced_symbols` (always present): Array of symbols (model:..., component:..., param:...) the expression references.
- notes: Model filter clauses are Condition nodes owned by the model (json_path starts with sources[...].filter); read raw_expr for the filter fields.

## Edge types

### Contains

Structural containment: page contains component, component contains component, model contains field.

- endpoints: Page -> Component; Component -> Component; Model -> Field
- field_path: never set (Not set.)
- meta: none
- when absent: Complete for what the scanner parsed.

### Triggers

A component owns an action (the action fires from that component).

- endpoints: Component -> Action
- field_path: never set (Not set.)
- meta: none
- when absent: Complete for what the scanner parsed.

### Reads

A component's expression reads a model field.

- endpoints: Component -> Field; Component -> Model
- field_path: always set (<model>.<field> that is read.)
- meta keys:
  - `actor_kind`: component.
  - `actor_id`: Component id.
  - `operation`: Reads.
  - `reason`: Why the edge exists.
  - `json_path`: Where in the page JSON the expression is.
  - `source_expr`: The expression text.
  - `source_field`: The property holding the expression (exp, value, ...).
  - `target_model`: Model read.
  - `target_field`: Field read.
  - `bare_symbol`: Set when the reference had no model prefix and was resolved through the data context.
  - `resolution`: How a bare symbol was resolved.
  - `target_model_path`: Path of the resolved model.
  - `data_context_component_id`: Data-context component used for resolution.
  - `data_context_dataSet`: Data set used for resolution.
  - `data_context_json_path`: JSON path used for resolution.
  - `data_context_source`: Source used for resolution.
- when absent: A component whose expressions reference no model or field has no Reads edge. A component bound through a data context may only show a Reads edge with a bare_symbol resolution.

### Writes

A component's submitField binds the component to a model field it writes.

- endpoints: Component -> Model
- field_path: always set (<model>.<field> that is written.)
- meta keys:
  - `actor_kind`: component | action | condition.
  - `actor_id`: Id of the actor within its page.
  - `operation`: The edge type name, or Conditions for a condition-to-owner edge.
  - `reason`: Human-readable sentence describing why the edge exists.
  - `source_expr`: Expression text the edge was derived from.
  - `target_model`: Model the edge refers to.
  - `target_field`: Field the edge refers to.
- when absent: Only components with a submitField produce it.

### FieldWrite

Field-level write: a component (via submitField) or an action writes a specific field. Usually accompanies Writes / ActionWrites, which point at the model.

- endpoints: Action -> Field; Component -> Field
- field_path: always set (<model>.<field> that is written.)
- meta keys:
  - `actor_kind`: component | action.
  - `actor_id`: Id of the writer.
  - `operation`: Writes.
  - `reason`: Why the edge exists.
  - `trigger`: Action trigger, on action writers.
  - `source_expr`: Value expression, when the write has one.
  - `target_model`: Model written.
  - `target_field`: Field written, when known.
- when absent: A write whose field cannot be resolved has only the model-level edge.

### ActionWrites

An action writes to a model (submitData, insertData, updateData, deleteData, ...).

- endpoints: Action -> Model
- field_path: always set (<model>.<field> that is written.)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionWrites.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `source_expr`: Value expression.
  - `target_model`: Model written.
  - `target_field`: Field written.
- when absent: Action types that write nothing have no such edge.

### ActionReads

An action reads a model or field (as a query input, a link parameter value or a set-parameter value).

- endpoints: Action -> Field; Action -> Model
- field_path: always set (<model>.<field> that is read.)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: Reads.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `source_expr`: Expression text.
  - `target_model`: Model read.
  - `target_field`: Field read.
- when absent: Only expressions that reference a model or field produce it.

### ActionNavigates

A link action opens another page.

- endpoints: Action -> Page
- field_path: always set (Path of the target page as written (may contain ../).)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionNavigates.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `target_model`: Target page name.
- when absent: A link whose target page cannot be resolved produces no edge here; the query layer reports it as UNRESOLVED_PAGE_NAVIGATION. Absence is not proof there is no navigation.

### PassesParam

A link action passes a value into a parameter of the target page.

- endpoints: Action -> Field
- field_path: always set (The value expression being passed (for example =model1.fieldA), not a model.field path.)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: PassesParam.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `target_field`: Parameter name.
  - `source_expr`: Value expression.
- when absent: Only link actions that carry parameters produce it.

### ActionSetsParam

A setParamValue action sets a page parameter.

- endpoints: Action -> Field
- field_path: always set (The value expression being assigned (for example =model1.fieldB).)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionSetsParam.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `target_field`: Parameter name.
  - `source_expr`: Value expression.
- when absent: Only setParamValue actions produce it.

### ActionControlsComponent

An action controls a UI component: opens or closes a dialog, shows or hides a component, switches a panel.

- endpoints: Action -> Component
- field_path: never set (Not set.)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionControlsComponent.
  - `trigger`: Event that fires the action.
  - `reason`: Says whether it opens, closes, shows, hides or switches.
  - `dialog`: Dialog id, for dialog actions.
  - `panel`: Panel id, for panel switches.
  - `panelbook`: Panel book id, for panel switches.
- when absent: Only UI-control action types produce it.

### ActionValidates

A validateData action validates a model field.

- endpoints: Action -> Model
- field_path: always set (<model>.<field> that is validated.)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionValidates.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `source_expr`: Expression text.
  - `target_model`: Model validated.
  - `target_field`: Field validated.
- when absent: Only validateData actions produce it.

### ActionLoadsData

A loadData / resetData style action loads or resets a model.

- endpoints: Action -> Model
- field_path: always set (<model>.* (the whole model).)
- meta keys:
  - `actor_kind`: action.
  - `actor_id`: Action id.
  - `operation`: ActionLoadsData.
  - `trigger`: Event that fires the action.
  - `reason`: Why the edge exists.
  - `target_model`: Model loaded.
- when absent: Only load / reset / refresh action types produce it.

### EmbedsPage

A component embeds another page (composition, not a user navigation).

- endpoints: Component -> Page
- field_path: always set (Path of the embedded page as written (may contain ../).)
- meta: none
- when absent: An embed whose target cannot be resolved produces no edge.

### DependsOn

Overloaded edge. operation = DependsOn: an expression depends on an upstream symbol (component value, parameter, user property, system variable, model field). operation = Conditions: a Condition node points at the owner it belongs to (Condition -> owner).

- endpoints: Component -> Component; Component -> Field; Condition -> Action; Condition -> Component; Condition -> Field; Condition -> Model
- field_path: always set (For operation = DependsOn: the referenced symbol (for example comp:input2.value, param:param1). For operation = Conditions: the json_path of the condition.)
- meta keys:
  - `actor_kind`: component | condition.
  - `actor_id`: Component id or condition id.
  - `operation`: DependsOn or Conditions.
  - `reason`: Says whether the condition belongs to its owner or depends on a symbol.
  - `json_path`: Location inside the page JSON.
  - `source_expr`: Expression text.
  - `source_field`: Property holding the expression (visibleCondition, exp, ...).
  - `source_file`: File of the referenced symbol, for cross-file references.
  - `target_component`: Referenced component id.
  - `target_param`: Referenced parameter name.
  - `condition_type`: On condition edges: the condition kind.
- when absent: Only symbols the expression parser recognises (component, param, user, system, model field) produce edges; other text is not tracked.

### DataflowInput

Dataflow lineage: a model takes another model as input (Model -> its input model).

- endpoints: Model -> Model
- field_path: always set (Path of the input table.)
- meta: none
- when absent: Model-level only; field-level lineage is not stored (see FieldAlias and the --explain verb).

### DataflowOutput

Dataflow lineage: a model outputs to another model (Model -> its output model).

- endpoints: Model -> Model
- field_path: always set (Path of the output table.)
- meta: none
- when absent: Model-level only; field-level lineage is not stored.

### OutputsTo

A dataflow table declares its output target. May point at the same model.

- endpoints: Model -> Model
- field_path: always set (Name of the output target.)
- meta: none
- when absent: Only dataflow tables with a declared output produce it.

### FieldAlias

Local model field to physical table field alias (field:model1.name -> field:table1.name).

- endpoints: Field -> Field
- field_path: always set (<local model>.<field> on the alias side.)
- meta: none
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
- **absent-edge-is-not-absent-relation**: A missing edge does not always mean the relation does not exist. Check the edge type's absent_when: unresolved navigation targets and unrecognised expression text produce no edge.
- **placeholder-model-path**: When a model was referenced but never resolved to a file, its node exists with a placeholder path '<name>.tbl'. Do not treat that path as a real file.
- **depends-on-is-overloaded**: DependsOn has two meanings, told apart by meta.operation: 'Conditions' means the Condition node belongs to its owner (Condition -> owner); 'DependsOn' means the source depends on the target symbol.
- **field-nodes-include-symbols**: Field nodes include page parameters (param:), user properties (user:) and system variables (system:), not only table fields (field:). meta.kind tells them apart.
- **field-path-varies**: r.field_path does not mean the same thing on every edge type: usually <model>.<field>, but an expression on PassesParam / ActionSetsParam, a page path on ActionNavigates / EmbedsPage, a table path on DataflowInput / DataflowOutput. See each edge type's field_path_meaning.
- **model-filters-are-conditions**: Filters configured on a model are Condition nodes owned by that model (DependsOn edge with meta.operation = Conditions, json_path starting with sources[...].filter). Read their raw_expr for the filter fields and values.

## GQL dialect notes

- Several edge types are written [:Reads|Triggers] (no colon before the second name).
- Use the CONTAINS / STARTS WITH operators; there is no =~ regex and no CONTAINS() function.
- Pattern predicates such as NOT (n)--() are not supported.
- meta is JSON text, not a map: return it to read it, filter it with CONTAINS; n.meta.exp is a syntax error.
- Always write (n:Node); the graph also holds internal IndexState records.
