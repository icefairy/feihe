#!/usr/bin/env python3
"""生成内置联想表 core/src/next_builtin.txt

## 数据从哪来

没有语料可用（在线下载在这台机器上只有几十 KB/s，中文维基 dump 3.4 GB——不现实），
所以这里走一条**不需要语料**的路：**从词库里的复合词反推词对**。

rime-ice 的 `base.dict.yaml` 有 54 万条词，其中大量是「词 + 词」的复合词：
`北京时间`、`北京大学`、`经济发展`、`人工智能`。把每个复合词按词库做**最短前缀**
切开，就是一条「前段 → 后段」的搭配，权重取那个复合词自己的词频。

这不是语言模型，也不需要是：联想要回答的是「打完『北京』接下来最可能是什么」，
而「北京时间 / 北京大学 / 北京市」正是这个词库统计出来的答案。它比 n-gram 少了
虚词搭配（「北京的」不在词库里），但那是用户自己的词对表该补的部分。

## 为什么要求两段都 ≥ 2 字

`北京市` 拆成 `北京|市` 会得到一堆「词 → 单个方位词」的噪音（北京→市、中国→省、
上海→区）。单字头的联想（`打` → 电话）同理，`不` / `的` 这类虚词会把表灌满。
两段都 ≥ 2 字是这张内置表的**质量下限**——单字搭配交给用户表去学。

## 参数怎么定的

`--heads` 5000：rime-ice 词频前 5000 的复合词头，覆盖日常写作的绝大部分。
再往上加，多出来的都是低频专名（「北京安盟信息技术」这类），只会污染联想。

用法：
  python tools/gen_next.py                 # 用已有缓存 _icebase.yaml
  python tools/gen_next.py --heads 8000    # 换规模
"""

import io
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "_icebase.yaml")
OUT = os.path.join(ROOT, "core", "src", "next_builtin.txt")

# 每个头词最多留几个后续。和 src/next.rs 的 MAX_TAIL 对齐（内置表可以更小一点：
# 用户表是「他真的这么打过」，内置表只是冷启动的垫子，5 个够用）。
TAIL = 5
# 复合词自身词频低于这个值就不要了：rime-ice 里大量专名的词频是 87 左右，
# 拆出来的都是「北京→安盟」这种没有任何联想价值的碎片。
MIN_WORD_FREQ = 3000


def is_han(c: str) -> bool:
    o = ord(c)
    return 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF or 0xF900 <= o <= 0xFAFF


def load(path: str) -> dict:
    """读 rime-ice 词库 → {词: 词频}，只留纯汉字词。"""
    freq = {}
    with io.open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#") or line.startswith("---"):
                continue
            parts = line.split("\t")
            if len(parts) < 3:
                continue  # name: / version: / sort: 这类头部键
            w, _, n = parts[0], parts[1], parts[2]
            try:
                n = int(n)
            except ValueError:
                continue
            if n <= 0 or not (2 <= len(w) <= 8):
                continue
            if not all(is_han(c) for c in w):
                continue
            freq[w] = max(freq.get(w, 0), n)
    return freq


def build(freq: dict, heads: int):
    """复合词 → 词对。返回 {头词: [(后续词, 权重)]}。"""
    pairs = {}
    for w, n in freq.items():
        if n < MIN_WORD_FREQ or len(w) < 4:
            continue
        # 最短前缀：`上海交通大学` 要切成 `上海|交通大学`（用户打完「上海」想接「交通大学」），
        # 不是 `上海交通|大学`（那个头词本身都很少单独打）。
        for i in range(2, len(w) - 1):
            a, b = w[:i], w[i:]
            if len(b) < 2:
                break  # i 再大也只会让 b 更短
            if a in freq and b in freq:
                pairs.setdefault(a, {})
                pairs[a][b] = pairs[a].get(b, 0) + n
                break

    # 只留「头词本身够常用」的，再按头词权重（自身词频 + 所有后续权重之和）取前 heads 个
    ranked = sorted(
        pairs.items(),
        key=lambda kv: (-(freq.get(kv[0], 0) + sum(kv[1].values())), kv[0]),
    )[:heads]

    out = {}
    for head, tails in ranked:
        top = sorted(tails.items(), key=lambda kv: (-kv[1], kv[0]))[:TAIL]
        if top:
            out[head] = top
    return out


def render(table: dict) -> str:
    lines = [
        "# 飞鹤输入法内置联想表",
        "# 由 tools/gen_next.py 从 rime-ice 词库（GPL-3.0）的复合词拆分生成，不要手改",
        "# 格式：头词<TAB>后续词 权重 后续词 权重 …（权重只用于排序，越大越常用）",
    ]
    for head in sorted(table, key=lambda h: (-sum(w for _, w in table[h]), h)):
        cells = " ".join(f"{w} {n}" for w, n in table[head])
        lines.append(f"{head}\t{cells}")
    return "\n".join(lines) + "\n"


def main():
    heads = 5000
    argv = sys.argv[1:]
    if "--heads" in argv:
        heads = int(argv[argv.index("--heads") + 1])
    if not os.path.exists(SRC):
        sys.exit(f"缺少 {SRC}（先跑 tools/gen_dict.py 把它下载下来）")

    freq = load(SRC)
    table = build(freq, heads)
    text = render(table)
    io.open(OUT, "w", encoding="utf-8", newline="\n").write(text)

    n_pairs = sum(len(v) for v in table.values())
    print(f"词库 {len(freq)} 条 → 头词 {len(table)} 个 / 词对 {n_pairs} 条")
    print(f"输出 {OUT}：{len(text.encode('utf-8')) / 1024:.0f} KB")
    print("样例：")
    for head in list(table)[:8]:
        print("  ", head, "→", " / ".join(w for w, _ in table[head]))


if __name__ == "__main__":
    main()
