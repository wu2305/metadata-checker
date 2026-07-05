#!/usr/bin/env python3
"""按 ## M / #### M36–M48 标题拆分 real-project-optimization-roadmap.md。"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "docs/real-project-optimization-roadmap.md"
OUT = ROOT / "docs/archive/roadmap"

SPLIT_RE = re.compile(
    r"^(?:## M[\w-]+：.+|#### M(?:36|37|38|39|40|41|42|43|44|45|46|47|48|97)：.+)$",
    re.MULTILINE,
)
HEADER_RE = re.compile(r"^(M[\w-]+)：(.+)$")


def slugify(m_id: str, title: str) -> str:
    mid = m_id.lower()
    words = re.findall(r"[A-Za-z][A-Za-z0-9]*", title)
    tail = "-".join(w.lower() for w in words[:6]) if words else "archive"
    return f"{mid}-{tail}"


def status_from(body: str) -> str:
    m = re.search(r"状态[：:]\s*([^\n。]+)", body)
    if not m:
        return "unknown"
    s = m.group(1).strip()
    if "已收敛" in s or "✅" in body[:200]:
        return "done"
    if "planned" in s.lower() or "规划" in s or "远期" in s:
        return "planned"
    if "起始" in s or "推进" in s:
        return "active"
    return "done" if "已" in s else "unknown"


def main() -> None:
    text = SRC.read_text(encoding="utf-8")
    matches = list(SPLIT_RE.finditer(text))
    if not matches:
        raise SystemExit("no sections found")

    OUT.mkdir(parents=True, exist_ok=True)
    index_rows: list[tuple[str, str, str, str]] = []

    for i, m in enumerate(matches):
        start = m.start()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        section = text[start:end].rstrip() + "\n"
        header_line = re.sub(r"^#+\s*", "", m.group(0).strip())
        hm = HEADER_RE.match(header_line)
        if not hm:
            continue
        m_id, title = hm.group(1), hm.group(2).strip()
        filename = slugify(m_id, title) + ".md"
        st = status_from(section)
        meta = (
            f"| milestone | {m_id} |\n"
            f"| status | {st} |\n"
            f"| archived_from | docs/real-project-optimization-roadmap.md |\n"
        )
        out_path = OUT / filename
        out_path.write_text(f"# {m_id}：{title}\n\n{meta}\n---\n\n{section.lstrip()}", encoding="utf-8")
        index_rows.append((m_id, title, filename, st))

    # M99 前言（## M99 之前、#### M36 之前的 M99 总述）
    m99 = next((r for r in index_rows if r[0] == "M99"), None)
    intro = text[: matches[0].start()].strip()
    if intro:
        (OUT / "_intro.md").write_text(intro + "\n", encoding="utf-8")

    def sort_key(row: tuple[str, str, str, str]) -> tuple[int, str]:
        mid = row[0]
        if mid == "M19-FIX":
            return (19, "FIX")
        m = re.match(r"M(\d+)", mid)
        return (int(m.group(1)) if m else 999, mid)

    index_rows.sort(key=sort_key)

    lines = [
        "# Roadmap 归档索引",
        "",
        "> 自 `docs/real-project-optimization-roadmap.md` 拆分（Phase C3）。活跃里程碑见 [milestones/INDEX.md](../../milestones/INDEX.md)。",
        "",
        "| id | title | status | file |",
        "|----|-------|--------|------|",
    ]
    for m_id, title, filename, st in index_rows:
        short = title.replace("|", "\\|")[:60]
        lines.append(f"| {m_id} | {short} | {st} | [{filename}]({filename}) |")

    (OUT / "README.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(index_rows)} sections to {OUT}")


if __name__ == "__main__":
    main()
