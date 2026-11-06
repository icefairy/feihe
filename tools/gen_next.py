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

## 为什么要分长度配额

最早只收 2 字头词（`北京市` 拆成 `北京|市` 会得到一堆「词 → 单个方位词」的噪音），
结果是**只有打完两字词才有联想**：单字（我 → 觉得）和 3 字词（计算机 → 网络）都没有。
但复合词里这两种搭配明明很多，只是被「按总权重取前 5000」挤掉了——2 字头词的数量
和权重都占优，长头词永远排不进去。

所以改成**按头词长度分桶、每桶各取前 N 个**：短头词不再把长头词挤死。

- 单字头词：`我觉得` → `我 → 觉得`、`打电话` → `打 → 电话`。这是这版新增的，
  权重按复合词词频之和排（单字本身不在 base.dict 里，字频从 `_ice8105.yaml` 只用来
  校验「这是个真汉字」）。虚词（的/了/是）确实会进来，但它们后面接什么本来就高频，
  而且表是「点一下才上屏」，噪音的代价低、漏联想的机会成本高。
- 3/4 字头词：`计算机 → 网络 / 软件 / 应用`、`上市公司 → 收购`。

## 为什么后续词仍要求 ≥ 2 字

`北京|市`、`中国|省`、`上海|区` 这种「词 → 单个方位词」是纯噪音：用户打完「北京」
不会想接一个「市」。单字后续交给用户表去学（用户真的那么打，才算数）。

## 参数怎么定的

见 `QUOTA`：单字 900 / 两字 5200 / 三字 800 / 四字 300，合计约 7200 个头词。
三、四字桶是「有多少要多少」（词库里能拆出来的就这么多），单字桶卡在 900——
再往下排都是生僻字（「鬻」开头的复合词），占了体积也用不上。

用法：
  python tools/gen_next.py                    # 用已有缓存 _icebase.yaml
  python tools/gen_next.py --quota 900,5200   # 只改前两档（其余沿用默认）
  python tools/gen_next.py --min-freq 5000    # 更严的词频门槛
"""

import io
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "_icebase.yaml")
# 单字字表：只用来判断「这个字是不是常用汉字」，不参与权重。
CHAR_SRC = os.path.join(ROOT, "_ice8105.yaml")
OUT = os.path.join(ROOT, "core", "src", "next_builtin.txt")

# 每个头词最多留几个后续。和 core/src/next.rs 的 MAX_TAIL 对齐（内置表可以更小一点：
# 用户表是「他真的这么打过」，内置表只是冷启动的垫子，5 个够用）。
TAIL = 5
# 复合词自身词频低于这个值就不要了：rime-ice 里大量专名的词频是 87 左右，
# 拆出来的都是「北京→安盟」这种没有任何联想价值的碎片。
MIN_WORD_FREQ = 3000
# 头词长度 → 最多收几个。索引 0 占位（没有 0 字头词）。
QUOTA = {1: 900, 2: 5200, 3: 800, 4: 300}


def is_han(c: str) -> bool:
    o = ord(c)
    return 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF or 0xF900 <= o <= 0xFAFF


def load(path: str) -> dict:
    """读 rime-ice 词库 → {词: 词频}，只留纯汉字词（2..=8 字，单字不在这个文件里）。"""
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


def load_chars(path: str) -> set:
    """读 8105 字表 → 合法单字集合。格式是 `字拼音权重`（没有分隔符）。"""
    chars = set()
    with io.open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            c = line[0]
            if is_han(c):
                chars.add(c)
    return chars


def build(freq: dict, chars: set, quota: dict):
    """复合词 → 词对。返回 {头词: [(后续词, 权重)]}。"""
    pairs = {}
    for w, n in freq.items():
        if n < MIN_WORD_FREQ or len(w) < 3:
            continue
        # 最短前缀：`上海交通大学` 要切成 `上海|交通大学`（用户打完「上海」想接「交通大学」），
        # 不是 `上海交通|大学`（那个头词本身都很少单独打）。
        for i in range(1, len(w) - 1):
            a, b = w[:i], w[i:]
            if len(b) < 2:
                break  # i 再大也只会让 b 更短
            if len(a) > 4:
                break
            # 单字头词：base.dict 里没有单字词条，用 8105 字表判断「这是个真字」。
            ok_head = (len(a) == 1 and a in chars) or (len(a) >= 2 and a in freq)
            if ok_head and b in freq:
                pairs.setdefault(a, {})
                pairs[a][b] = pairs[a].get(b, 0) + n
                break

    # 按头词长度分桶，各桶按「自身词频 + 所有后续权重之和」取前 N 个。
    # 不分桶的话 2 字头词会把 1/3/4 字头词全挤掉——那正是旧版「只有两字有联想」的根因。
    out = {}
    for length in sorted(quota):
        cap = quota[length]
        bucket = [(h, t) for h, t in pairs.items() if len(h) == length]
        bucket.sort(key=lambda kv: (-(freq.get(kv[0], 0) + sum(kv[1].values())), kv[0]))
        for head, tails in bucket[:cap]:
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
    argv = sys.argv[1:]
    quota = dict(QUOTA)
    if "--quota" in argv:
        vals = argv[argv.index("--quota") + 1].split(",")
        for i, v in enumerate(vals, start=1):
            if i in quota:
                quota[i] = int(v)
    global MIN_WORD_FREQ
    if "--min-freq" in argv:
        MIN_WORD_FREQ = int(argv[argv.index("--min-freq") + 1])

    if not os.path.exists(SRC):
        sys.exit(f"缺少 {SRC}（先跑 tools/gen_dict.py 把它下载下来）")
    if not os.path.exists(CHAR_SRC):
        sys.exit(f"缺少 {CHAR_SRC}（单字表，用来校验单字头词）")

    freq = load(SRC)
    chars = load_chars(CHAR_SRC)
    table = build(freq, chars, quota)
    text = render(table)
    io.open(OUT, "w", encoding="utf-8", newline="\n").write(text)

    n_pairs = sum(len(v) for v in table.values())
    by_len = {}
    for h in table:
        by_len[len(h)] = by_len.get(len(h), 0) + 1
    print(f"词库 {len(freq)} 条 / 单字 {len(chars)} 个 → 头词 {len(table)} 个 / 词对 {n_pairs} 条")
    print("按头词长度：" + "  ".join(f"{k} 字 {v}" for k, v in sorted(by_len.items())))
    print(f"输出 {OUT}：{len(text.encode('utf-8')) / 1024:.0f} KB")
    for L in sorted(by_len):
        sample = [h for h in table if len(h) == L][:3]
        for h in sample:
            print(f"  [{L} 字] {h} → " + " / ".join(w for w, _ in table[h]))


if __name__ == "__main__":
    main()
