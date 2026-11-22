# M59 Grafeo source-facts fixtures

这些静态文件是人工审阅的原始输入；测试 expected 值不从 scanner 或 redb 生成。

## 初始源与 gold

| 文件 / JSON 路径 | 源事实 | 预期图事实 |
|---|---|---|
| <code>app/a.spg</code> <code>/referenceResources/0</code> | 第 0 项引用 <code>b.spg</code> | 项目路径解析为 <code>app/b.spg</code> |
| <code>app/a.spg</code> <code>/canvas</code> | 根组件 <code>canvas</code> | <code>page:app/a.spg</code> 包含 <code>comp:app/a.spg\|canvas</code> |
| <code>app/a.spg</code> <code>/canvas/components/0</code> | <code>embedsuperpage</code>，id <code>embed_b</code>，<code>resPath</code> 为 0 | <code>comp:app/a.spg\|canvas</code> 包含 <code>comp:app/a.spg\|embed_b</code>；embed 以 <code>EmbedsPage</code> 指向 <code>page:app/b.spg</code>，<code>field_path</code> 为 <code>app/b.spg</code> |
| <code>app/a.spg</code> <code>/canvas/components/1</code> | <code>text</code>，id <code>label_a</code>，value <code>A</code> | 根 canvas 包含该组件；meta 保留类型、JSON 路径和值 |
| <code>app/b.spg</code> <code>/canvas</code> 与 <code>/canvas/components/0</code> | 根 canvas 和独有 text 组件 <code>label_b</code>，value <code>B</code> | <code>page:app/b.spg</code>、<code>comp:app/b.spg\|canvas</code>、<code>comp:app/b.spg\|label_b</code>；页面、根 canvas 和子组件的 Contains 关系 |
| <code>db/orders.tbl</code> <code>/dimensions/0</code> | 逻辑字段 <code>订单号</code>，物理 <code>dbfield</code> 为 <code>ORDER_NO</code>，类型 <code>C</code>，长度 24 | <code>field:orders.订单号</code>，path <code>db/orders.tbl</code>，meta 保留这些原始字段属性 |
| <code>db/orders.tbl</code> <code>/dimensions/1</code> | 逻辑字段 <code>含税金额</code>，物理 <code>dbfield</code> 为 <code>TAX_AMOUNT</code>，类型 <code>N</code>，长度 16 | <code>field:orders.含税金额</code>，path <code>db/orders.tbl</code>，meta 保留这些原始字段属性 |

初始完整节点集合为上述 2 个 Page、5 个 Component、<code>model:orders</code> 和 2 个 Field。
每个 Component 都由其 Page 直接包含；有父组件时，还应有父 Component 到子 Component
的 Contains 边。另有 <code>model:orders</code> 到两个 Field 的 Contains 边，以及表中所述
EmbedsPage 边。测试逐项比较完整节点及边 tuple，不把总数或 contains 检查当作全集判据。
没有 <code>dbTableName</code> 是有意选择：本夹具不声明物理输出表，避免把输出表语义混入此 slice。

## 增量源

<code>app/b-updated.spg</code> 只在首次扫描之后复制为项目中的 <code>app/b.spg</code>；
seed 不扫描更新版夹具。更新源内组件 id 从 <code>label_b</code> 改为
<code>label_b_v2</code>，value 改为更长的 <code>B after source edit</code>。预期旧组件完全消失，新组件
出现；Page B、embed 关系、其余源事实和本表的全部 Contains 边仍满足人工 gold。

## 来源边界

这些 gold 以源文件项目相对路径、节点 path、组件 JSON 路径及原始属性为证据。
scanner 旧兼容路径通过 <code>add_node</code> / <code>add_edge_with_meta</code> 写入 <code>origin_file=None</code>；
本 fixture **不**把该 None 当作正确来源，也不宣称 origin ownership 已通过。完整
origin_file 归属、所有权重建和 project-scoped identity 是后续 gate。

本夹具只涵盖扫描器当前支持的 .spg / .tbl 来源。Script schema 已登记但 scanner
尚未摄入脚本；脚本缺边不代表没有脚本或关系。F3/F5/F6、Graph-first L1–L5、
Script 摄入及 M59-PATH 的完整判定仍按 M59 gate 跟踪。
