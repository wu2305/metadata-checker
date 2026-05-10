use serde::Deserialize;
use serde_json;

// ============================================================
// JSON 反序列化结构体
// ============================================================

#[derive(Debug, Deserialize)]
pub struct RawSuperPage {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub params: Vec<RawParam>,
    #[serde(default)]
    pub sources: Vec<RawSource>,
    #[serde(rename = "referenceResources", default)]
    pub reference_resources: Vec<String>,
    #[serde(default)]
    pub canvas: Option<RawComponent>,
}

#[derive(Debug, Deserialize)]
pub struct RawParam {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawSource {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "modelType", default)]
    pub model_type: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawAction {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "actionType", default)]
    pub action_type: String,
    #[serde(rename = "triggerType", default)]
    pub trigger_type: String,
    #[serde(default)]
    pub submit_range: Option<String>,
    #[serde(rename = "submitComponent", default)]
    pub submit_component: Vec<String>,
    #[serde(rename = "dataSet", default)]
    pub data_set: Option<serde_json::Value>,
    #[serde(rename = "dataRange", default)]
    pub data_range: Option<String>,
    #[serde(rename = "fieldValues", default)]
    pub field_values: Vec<RawFieldValue>,
    #[serde(default)]
    pub path: Option<serde_json::Value>,
    #[serde(rename = "shortUrl", default)]
    pub short_url: Option<String>,
    #[serde(default)]
    pub data: Vec<RawLinkParam>,
    #[serde(default)]
    pub params: Vec<RawSetParam>,
    #[serde(rename = "targetType", default)]
    pub target_type: String,
    #[serde(rename = "waitPrev", default)]
    pub wait_prev: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(rename = "conditionExp", default)]
    pub condition_exp: Option<String>,
    #[serde(rename = "targetComponent", default)]
    pub target_component: Vec<String>,
    #[serde(default)]
    pub panelbook: Option<String>,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub dialog: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawFieldValue {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "valueType", default)]
    pub value_type: String,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawLinkParam {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawSetParam {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawComponent {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub component_type: String,
    #[serde(rename = "resPath", default)]
    pub res_path: Option<serde_json::Value>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(rename = "defaultValue", default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(rename = "defaultValueExp", default)]
    pub default_value_exp: Option<serde_json::Value>,
    #[serde(default)]
    pub visible: Option<serde_json::Value>,
    #[serde(default)]
    pub exp: Option<serde_json::Value>,
    #[serde(default)]
    pub enable: Option<serde_json::Value>,
    #[serde(rename = "disableCondition", default)]
    pub disable_condition: Option<serde_json::Value>,
    #[serde(default)]
    pub text: Option<serde_json::Value>,
    #[serde(default)]
    pub formula: Option<serde_json::Value>,
    #[serde(default)]
    pub html: Option<serde_json::Value>,
    #[serde(rename = "calcCondition", default)]
    pub calc_condition: Option<serde_json::Value>,
    #[serde(rename = "itemFilter", default)]
    pub item_filter: Option<serde_json::Value>,
    #[serde(rename = "validExp", default)]
    pub valid_exp: Option<serde_json::Value>,
    #[serde(rename = "calcExp", default)]
    pub calc_exp: Option<serde_json::Value>,
    #[serde(rename = "maskCondition", default)]
    pub mask_condition: Option<serde_json::Value>,
    #[serde(rename = "submitCondition", default)]
    pub submit_condition: Option<serde_json::Value>,
    #[serde(rename = "visibleCondition", default)]
    pub visible_condition: Option<serde_json::Value>,
    #[serde(rename = "submitPageCondition", default)]
    pub submit_page_condition: Option<serde_json::Value>,
    #[serde(rename = "defaultPanelCondition", default)]
    pub default_panel_condition: Option<serde_json::Value>,
    #[serde(default)]
    pub desc: Option<serde_json::Value>,
    #[serde(default)]
    pub placeholder: Option<serde_json::Value>,
    #[serde(default)]
    pub url: Option<serde_json::Value>,
    #[serde(rename = "documentTitle", default)]
    pub document_title: Option<serde_json::Value>,
    #[serde(rename = "inputTitle", default)]
    pub input_title: Option<serde_json::Value>,
    #[serde(rename = "labelValue", default)]
    pub label_value: Option<serde_json::Value>,
    #[serde(rename = "panelName", default)]
    pub panel_name: Option<serde_json::Value>,
    #[serde(rename = "rootPath", default)]
    pub root_path: Option<serde_json::Value>,
    #[serde(rename = "selectedCaption", default)]
    pub selected_caption: Option<serde_json::Value>,
    #[serde(rename = "confirmCaption", default)]
    pub confirm_caption: Option<serde_json::Value>,
    #[serde(default)]
    pub tip: Option<serde_json::Value>,
    #[serde(default)]
    pub badge: Option<serde_json::Value>,
    #[serde(default)]
    pub count: Option<serde_json::Value>,
    #[serde(rename = "attrCaption", default)]
    pub attr_caption: Option<serde_json::Value>,
    #[serde(default)]
    pub caption: Option<serde_json::Value>,
    #[serde(rename = "defaultSelect", default)]
    pub default_select: Option<serde_json::Value>,
    #[serde(rename = "defaultCheck", default)]
    pub default_check: Option<serde_json::Value>,
    #[serde(rename = "maxLevel", default)]
    pub max_level: Option<serde_json::Value>,
    #[serde(rename = "submitField", default)]
    pub submit_field: Option<String>,
    #[serde(rename = "submitData", default)]
    pub submit_data: Option<bool>,
    #[serde(default)]
    pub actions: Vec<RawAction>,
    #[serde(default)]
    pub components: Vec<RawComponent>,
    #[serde(default)]
    pub panels: Vec<RawComponent>,
    #[serde(default)]
    pub steps: Vec<RawComponent>,
    #[serde(default)]
    pub comps: Vec<RawComponent>,
}

// ============================================================
// 对外暴露的公共类型
// ============================================================
