# 2026-09-08 归档分支历史版本核对

这是 [原始清单](branch-cleanup-2026-09-08.md) 的后续证据，不改写原始 166/171 快照。
核对 main 固定为 `9adb5d0`；12 个本地 `archive/*` tag 保留。
没有据此恢复旧实现、删除 tag 或宣布所有旧行为均在当前 main 中保留。

## 精确版本证据

原待复核的 171 个「分支、路径」组合中，128 项的归档末态 mode/blob 在相同路径的
main 可达历史中出现过；另 43 项没有找到这种精确版本证据。
机器清单见 [JSON](branch-retention-history-2026-09-08.json)，逐项记录归档提交、
path、mode/blob、证据提交及状态。

历史候选通过 `git log --raw --full-history --no-renames` 找出，再**逐项独立验证**：

1. 证据提交是固定 main 的祖先；不是只在另一个归档分支中出现。
2. 证据提交的完整树中，同一路径的 mode/blob 与原始清单的归档版本完全相等。
3. 本地 tag 仍指向原记录的归档提交；全部 171 个组合恰好覆盖一次，无漏项或重复。

这个结论只证明 **128 份历史版本已进入 main 历史且仍可恢复**，不证明当前实现行为等价，
也不证明后续删除或重写的意图正确。其余 43 项的版本差异同样不等于内容丢失。
原有166项的计数仍指原基线 `be23369`，没有伪装成一次新基线的全量验收。

## 重放历史证据

在持有上述归档对象的仓库根目录执行以下只读核验；无需编译。原清单 hash 和 main
完整 SHA 已写入 JSON。Git 对象若丢失，报告文本不能替代备份。

```python
import hashlib
import json
import subprocess
from pathlib import Path

root = Path('docs/governance')
report = json.loads((root / 'branch-retention-history-2026-09-08.json').read_text())
original_bytes = (root / report['input']).read_bytes()
assert hashlib.sha256(original_bytes).hexdigest() == report['input_sha256']
original = json.loads(original_bytes)
pending = {(archive['tag'], change['path']): (archive, change)
           for archive in original['archives'] for change in archive['changes']
           if change['status'] == 'requires_content_review'}
assert len(report['records']) == len(pending) == 171
assert {(item['tag'], item['path']) for item in report['records']} == set(pending)
verified = 0
for item in report['records']:
    archive, change = pending[(item['tag'], item['path'])]
    tip = subprocess.check_output(['git', 'rev-parse', item['tag'] + '^{commit}'], text=True).strip()
    assert tip == item['archive_commit'] == archive['commit']
    assert item['archive_entry'] == change['archive']
    if item['status'] != 'exact_version_in_main_history':
        assert item['status'] == 'requires_patch_review'
        continue
    commit = item['evidence_commit']
    subprocess.run(['git', 'merge-base', '--is-ancestor', commit, report['main_sha']], check=True)
    tree = {}
    for entry in subprocess.check_output(['git', 'ls-tree', '-rz', commit]).split(b'\0'):
        if entry:
            metadata, path = entry.split(b'\t', 1)
            mode, kind, blob = metadata.decode().split()
            tree[path.decode()] = [mode, blob]
    assert tree.get(item['path']) == item['archive_entry']
    verified += 1
assert verified == report['counts']['exact_version_in_main_history'] == 128
assert len(pending) - verified == report['counts']['requires_patch_review'] == 43
print('128 historical versions verified; 43 require patch review')
```

## 内容级核查

43 项无精确历史版本证据的差异中，本轮另确认 1 项为**明确未合并、延期评估的原型**，
其余 **42 项未完成内容级严证**。JSON 的 128/43 是上述历史树核对分类，
不因这项人工判断改写；128 项的当前行为等价性也没有因此获验收。

| 分支 / 路径 | 结论 | 可核查依据 |
|---|---|---|
| `archive/m52-performance-optimization` / `src/page_logic_fragments.rs` | 未合并的 native 文件后端原型；保留归档，按原计划延期，不自动恢复 | 归档提交 `60dacc7eda15c0f997c3daa65bbfdf0a9d38bb36` 中 `PageLogicFragmentStore` 写入 availability/prerequisites/paths/warm_stages/meta，并按 schema/fingerprint 校验读取；main `9adb5d0` 无该文件。M53 journal 在该 main 的第 3、203 行明确 fragment 工作移出里程碑、旧 spike 在未合并分支；第 70–78 行记录其原型范围 |

这不是「当前已有其他实现完整替代」的证据。M53 journal 前文原写“已实现 native
文件后端 spike”，容易脱离文末限制误读，本轮已将该节标成历史原型、未合并。
参见 [M53 记录](../milestones/performance/m53-performance-continuation.md)。

剩余项须比较 merge-base → 归档提交改动，再核对 main 中对应调用链和断言。
不能以文件存在、测试函数名相同或整个旧文件与新文件差异很大代替这一步。
本轮只读协作审阅给出的其他聚合“retained/enhanced”结论没有附足够逐项证据，
主线程未将其计入已核验结果。

PR36 合并后续查见 [逐项改动复核](branch-retention-patches-2026-09-08.md)：另核查24项，
恢复1份历史评测草稿；该轮结束时18项未决。本页的128/43和上述42项是前轮快照，不回写。
