# PR 描述模板

复制到 CNB PR body：

```markdown
## Summary
<!-- 1–3 句：为什么改、改什么 -->

## Test plan
- [ ] `cargo test ...`
- [ ] （其它命令）

## Docs
<!-- 列出变更的 docs 路径；无则写「无」 -->
-

## Baseline impact
<!-- yes / no；若 yes，写明 CI build sn 与受影响 bench -->
no
```
