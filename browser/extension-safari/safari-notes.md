# M43.9 Safari 兼容备注

## 目标约束

- Safari 走同一套 M43 extension core（`extension-core` + bridge protocol），不再新增独立业务逻辑。
- Safari Web Extension 兼容路径偏人工，**不在自动化中自动启动/运行 Safari**。
- 发现 Safari 不支持的能力时，必须返回 stable diagnostic，而不是直接静默报错。

## Safari 与 Chromium 差异（需在 runbook 中显式记录）

- **权限模型**  
  - Chromium 常见 `host_permissions` 与 `activeTab` 组合行为在 Safari 下可能不完全一致。建议仅声明必要 host，避免过度权限。
- **生命周期模型**  
  - Safari 对 service worker/persistent background 的行为更严苛，部分版本对某些生命周期回调有差异。若出现不一致，必须在 diagnostic 中记录可复现原因。
- **注入能力差异**  
- `chrome.scripting` 与特定高权限 API 在 Safari 可能缺失或行为退化。出现缺失时应返回统一诊断，不要尝试降级到不可见失败分支。
- **调试对象差异**  
  - Safari 的 Content Script / Background 页面调试入口与 Chromium 不同，日志采集不能直接套用 Chrome DevTools 脚本路径。
- **Manifest 字段兼容**  
  - 避免使用 Chrome Web Store 专属字段；manifest 以 `manifest.template.json` 为准，保留最小字段集。

建议诊断码（可按项目命名风格统一）：

- `SAFARI_API_UNAVAILABLE`
- `SAFARI_EXTENSION_WORKER_UNSUPPORTED`
- `SAFARI_PAGE_SCRIPT_BRIDGE_TIMEOUT`
- `SAFARI_STABLE_DIAGNOSTIC`

## Safari 手工开启流程（不自动化）

1. 准备可用的 Safari staging 目录（建议先生成共享 core 的基础包）。
2. 使用 macOS 的 Safari Web Extension 转换器或 Xcode 工程导入该 staging 包。
3. 在 Safari `设置 -> 扩展` 启用开发模式和调试。
4. 选择目标网站访问权限（至少覆盖真实 BI host），并允许该扩展在该域名运行。
5. 重新加载目标 BI 页面。

## 调试步骤

### 页面侧

```js
function marker(name) {
  const key = `data-metadata-checker-${name}`;
  return document.querySelector(`[${key}]`)?.getAttribute(key) ?? null;
}

console.table({
  bridge: marker("bridge"),
  bridgeProtocol: marker("bridge-protocol"),
  bridgeStatus: marker("bridge-status"),
});
```

- 确认 `__metadata_checker_designer_ready__` 事件是否触发；
- 确认 `marker("bridge")`、`marker("bridge-status")` 与 `marker("bridge-protocol")` 合理；
- 若无 bridge，popup/页面应返回 `SAFARI_*` 类 stable diagnostic。

### Extension 调试侧

1. 在 Safari 打开扩展调试器（Extension Builder / Debug inspector）。
2. 分别打开：
   - content script 实例；
   - background/extension runtime 实例；
   - popup 实例（若存在）。
3. 关注：
   - 收到的 `onInitDesigner`/ready 事件；
   - 发送/接收 selection snapshot 是否成功；
   - `Analyze current selection` 是否触发稳定回执；
   - 是否出现 `SAFARI_*` 诊断码。

## 常见限制与验收要求

- Safari 下若 API 不支持，必须**可复现、可归档、可读**，并返回稳定诊断码。
- 不要求在 Safari 下完成全链路图面板回归，首轮以 bridge + selection + analyze request 的稳定性为准。
- 所有环境限制（权限拒绝、站点不在白名单、API 不可用）应在验收记录中作为人工限制项单独列明。
