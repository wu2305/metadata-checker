mod answer_facts;
mod conditions;
mod path_partition;
mod value_source;

pub(crate) use answer_facts::build_answer_facts;
pub(super) use answer_facts::{
    build_context_summary, build_primary_reason_for_intent, build_traversal_policy,
};
pub(crate) use conditions::{
    ConditionCollectorCache, ConditionFact, annotate_condition_scope, classify_condition,
    collect_condition_facts_for_node, collect_conditions_for_node, condition_owned_by_node,
    dedupe_conditions, expand_total_row_count_gates, precollect_all_node_conditions,
};
pub(super) use conditions::{
    collect_inherited_conditions_by_json_path, component_ancestor_chain, component_json_paths,
};
pub(super) use path_partition::partition_paths_for_intent;
pub(super) use value_source::build_value_source_context;
