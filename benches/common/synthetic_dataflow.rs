use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::scanner::process_tbl_file_from_string;
use serde_json::json;

/// 构造带多 Join / Union / Filter / Output 字段的 DataFlow 病理图。
pub fn build_pathological_dataflow_graph(
    output_field_count: usize,
    join_layers: usize,
    filter_count: usize,
) -> (MemoryGraphStore, String) {
    let mut nodes = serde_json::Map::new();
    nodes.insert(
        "src1".to_string(),
        json!({
            "id": "src1",
            "alias": "源表A",
            "type": "ModelTable",
            "moduleTablePath": "$DATA:/bench/source_a.tbl"
        }),
    );
    nodes.insert(
        "src2".to_string(),
        json!({
            "id": "src2",
            "alias": "源表B",
            "type": "ModelTable",
            "moduleTablePath": "$DATA:/bench/source_b.tbl"
        }),
    );

    let mut previous = "src1".to_string();
    for layer in 0..join_layers {
        let join_id = format!("join{layer}");
        let right = if layer % 2 == 0 {
            "src2".to_string()
        } else {
            format!("filter{layer}")
        };
        nodes.insert(
            join_id.clone(),
            json!({
                "id": join_id,
                "alias": format!("关联{layer}"),
                "type": "Join",
                "inputNodes": [previous, right],
                "joinConditions": [{
                    "joinType": "left",
                    "leftTable": previous,
                    "rightTable": right,
                    "clauses": [{
                        "leftExp": "id",
                        "operator": "=",
                        "rightExp": "id"
                    }]
                }]
            }),
        );
        previous = join_id;
    }

    for filter_idx in 0..filter_count {
        let filter_id = format!("filter{filter_idx}");
        nodes.insert(
            filter_id.clone(),
            json!({
                "id": filter_id,
                "alias": format!("过滤{filter_idx}"),
                "type": "Filter",
                "inputNodes": [previous],
                "filter": {
                    "clauses": [{
                        "leftExp": "status",
                        "operator": "=",
                        "rightValue": format!("open_{filter_idx}")
                    }]
                }
            }),
        );
        previous = filter_id;
    }

    let union_id = "union1".to_string();
    nodes.insert(
        union_id.clone(),
        json!({
            "id": union_id,
            "alias": "合并",
            "type": "Union",
            "inputNodes": [previous, "src2"],
            "unionMap": [{
                "visible": true,
                "values": ["id", "id"]
            }]
        }),
    );

    let output_fields: Vec<_> = (0..output_field_count)
        .map(|idx| {
            json!({
                "name": format!("字段{idx}"),
                "dbfield": format!("field_{idx}"),
                "exp": format!("field_{idx}")
            })
        })
        .collect();
    nodes.insert(
        "output1".to_string(),
        json!({
            "id": "output1",
            "alias": "病理输出",
            "type": "Output",
            "inputNodes": [union_id],
            "fields": output_fields
        }),
    );

    let tbl = json!({
        "version": "1.0",
        "properties": {
            "name": "bench_pathology",
            "dbTableName": "fact_bench_pathology",
            "dbSchema": "public"
        },
        "dimensions": (0..output_field_count).map(|idx| {
            json!({
                "name": format!("字段{idx}"),
                "dbfield": format!("field_{idx}"),
                "dataType": "C",
                "length": 50,
                "isDimension": true,
                "exp": format!("field_{idx}")
            })
        }).collect::<Vec<_>>(),
        "dataFlow": { "nodes": nodes }
    });
    let tbl_text = serde_json::to_string(&tbl).expect("serialize pathology dataflow tbl");
    let logical_path = "bench/pathology.tbl";
    let mut store = MemoryGraphStore::new();
    process_tbl_file_from_string(&mut store, logical_path, &tbl_text)
        .expect("build pathology dataflow graph");
    let model_id = "model:pathology".to_string();
    (store, model_id)
}
