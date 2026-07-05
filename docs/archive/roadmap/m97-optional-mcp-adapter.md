# M97：Optional MCP Adapter（远期可选）

| milestone | M97 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M97：Optional MCP Adapter（远期可选）

触发条件：只有当 Zed Agent、Claude Desktop、Cursor 等多个 MCP client 明确需要复用 metadata-checker，且浏览器/CLI/stdio 主线已经稳定时，才恢复 MCP 适配。

当前结论：

- MCP 不进入 M42。
- 不实现 MCP tools/resources。
- 不做偏文本向 tool result 优化。
- 不为 Zed 集成提前背 MCP 协议维护成本。

可选任务（暂不实施）：

- MCP stdio server spike。
- `metadata_query` 最小 tool。
- Zed `context_servers` 配置验证。
- MCP tool 输出复用 runtime contract。
- MCP 不暴露 shell、不返回 secret、不绕过 runtime 直接读 raw graph。
