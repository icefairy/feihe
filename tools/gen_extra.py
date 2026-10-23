#!/usr/bin/env python
"""把 `tools/wordlists/*.txt` 里的候选词并入词库（`src/extra_*.txt`）。

## 为什么要有这个脚本

`src/extra_dict.txt` 是手工挑的补充词（缺一个就手写一行）。几万条的量靠手写既不
现实，也没法保证拼音对——而**拼音错 = 这个词永远打不出来**：词库存的是全拼，
运行时按当前方案现场编码，拼音写错就是码写错。

所以流程拆成两半：

1. 词表（`tools/wordlists/*.txt`，一行一个词，可以 `#` 注释）——纯文本，人改得动；
2. 拼音（本脚本用 pypinyin 现算）——机器算，不手抄。

## 拼音校验（重要）

pypinyin 的**词级**多音字判断很准（`银行`→yin hang、`重庆`→chong qing、`六安`→lu an），
但生僻字的**字级**读音会翻车（`川芎` 给成 chuan qiong，实际 chuan xiong）。所以：

- 以 `src/dict.txt` 里单字条目的读音为准（那是人核对过的）；
- pypinyin 给的读音不在该字的读音集合里 → 记进 `_review.txt` 人工看，
  同时按下面的规则兜底：该字只有一个读音就用那个，有多个就保留 pypinyin 的判断；
- 字不在 `dict.txt` 里（编码表里可能没有）→ 整词丢弃，记进 `_dropped.txt`。

确切的错词用 `tools/pinyin_overrides.txt`（`词<TAB>全拼`）钉死，优先级最高。

## 用法

    python tools/gen_extra.py            # 只报告，不写文件
    python tools/gen_extra.py --write    # 写 src/extra_<分类>.txt
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WL = os.path.join(ROOT, "tools", "wordlists")
OVERRIDES = os.path.join(ROOT, "tools", "pinyin_overrides.txt")

# 分类 → (输出文件, 最短, 最长)
# 词长限制不是洁癖：`common` 的用户要求是 2~4 字，长词归到各自分类里。
# 顺序即优先级：专有名词分类在前，`common` 最后捡剩下的。
# 同一个词只进一个文件（先到先得），否则 `服务器` 会同时出现在 it 和 common 里。
CATS = {
    "tcm_herb": ("src/extra_tcm_herb.txt", 2, 5),
    "tcm_patent": ("src/extra_tcm_patent.txt", 3, 6),
    "drug": ("src/extra_drug.txt", 2, 8),
    "place": ("src/extra_place.txt", 2, 6),
    "it": ("src/extra_it.txt", 2, 5),
    # 术数类。词都不长（最长「旺相休囚死」5 字），但专名多、生僻读音多，
    # 读音一律在 tools/pinyin_overrides.txt 里钉，别指望 pypinyin 猜对。
    "ganzhi": ("src/extra_ganzhi.txt", 2, 5),
    "yijing": ("src/extra_yijing.txt", 2, 5),
    "shensha": ("src/extra_shensha.txt", 2, 5),
    "qimen": ("src/extra_qimen.txt", 2, 5),
    "liuren": ("src/extra_liuren.txt", 2, 5),
    "liuyao": ("src/extra_liuyao.txt", 2, 5),
    "common": ("src/extra_common.txt", 2, 4),
}

CJK = re.compile(r"^[\u4e00-\u9fff]+$")
SYL = re.compile(r"^[a-z]+$")


def read_lines(path):
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            yield line


def load_known():
    """词库里已经有词。新词只补缺的，绝不覆盖。

    **只认人手写的那几份**（`dict.txt` / `extra_dict.txt` / `user_dict.txt`）：
    `src/extra_<分类>.txt` 是本脚本的产物，拿它当依据会让第二次运行报告「全是旧词」，
    而且一旦某个词被脚本过滤掉就再也补不回来。产物每次整份重写。
    """
    known = set()
    for base in ("src/dict.txt", "src/extra_dict.txt", "src/user_dict.txt"):
        path = os.path.join(ROOT, base)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                if "\t" not in line:
                    continue
                w = line.split("\t", 1)[0].strip()
                if w:
                    known.add(w)
    return known


def load_char_readings():
    """单字 → 该字在词库里登记过的读音（有序，第一条是词库里的常用读音）。

    只取 `dict.txt`：那是人核对过的表，且它就是编码的依据。`extra_*` 是我们的
    产物，拿它当依据会自己证明自己。
    """
    readings = {}
    path = os.path.join(ROOT, "src", "dict.txt")
    with open(path, encoding="utf-8") as f:
        for line in f:
            if "\t" not in line:
                continue
            w, py = line.rstrip("\n").split("\t", 1)
            if len(w) != 1:
                continue
            readings.setdefault(w, []).append(py.strip())
    return readings


def load_overrides():
    m = {}
    for line in read_lines(OVERRIDES):
        if "\t" not in line:
            continue
        w, py = line.split("\t", 1)
        m[w.strip()] = py.strip()
    return m


def main():
    write = "--write" in sys.argv
    from pypinyin import lazy_pinyin

    known = load_known()
    readings = load_char_readings()
    overrides = load_overrides()
    review, dropped = [], []

    stats = {}
    total_new = 0
    used = set()
    for cat, (out_rel, lo, hi) in CATS.items():
        # 一个分类可以由多个分片组成（`common_01.txt`、`common_02.txt`…），
        # 分批往词表里加词时不用把上万个词挤进一个文件。
        srcs = sorted(
            p
            for p in glob.glob(os.path.join(WL, f"{cat}*.txt"))
            if not os.path.basename(p).startswith("_")
        )
        words = [w for p in srcs for w in read_lines(p)]
        out, seen = [], set()
        for w in words:
            if w in known or w in seen or w in used:
                continue
            if not CJK.match(w) or not (lo <= len(w) <= hi):
                continue
            seen.add(w)

            if w in overrides:
                py = overrides[w]
            else:
                toks = lazy_pinyin(w)
                if len(toks) != len(w) or not all(SYL.match(t) for t in toks):
                    dropped.append((w, "音节异常: " + " ".join(toks)))
                    continue
                fixed = []
                for ch, t in zip(w, toks):
                    rd = readings.get(ch)
                    if rd is None:
                        fixed = None
                        break
                    if t in rd:
                        fixed.append(t)
                    elif len(rd) == 1:
                        fixed.append(rd[0])
                        review.append((w, ch, t, rd[0]))
                    else:
                        fixed.append(t)
                        review.append((w, ch, t, "/".join(rd)))
                if fixed is None:
                    dropped.append((w, "字不在词库里"))
                    continue
                py = " ".join(fixed)

            out.append((w, py))

        used.update(w for w, _ in out)
        stats[cat] = (len(words), len(out))
        total_new += len(out)
        if write and out:
            path = os.path.join(ROOT, out_rel)
            header = (
                f"# 自动生成的补充词表（{cat}）\n"
                "# 由 tools/gen_extra.py 从 tools/wordlists/%s*.txt 生成，"
                "格式：词<TAB>全拼\n"
                "# 只收词库里原本没有的词；改词请改 wordlists 里的源文件再重跑，不要手改本文件。\n"
                % cat
            )
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8", newline="\n") as f:
                f.write(header)
                for w, py in out:
                    f.write(f"{w}\t{py}\n")

    print(f"{'分类':<12}{'候选':>8}{'新增':>8}")
    for cat, (n, k) in stats.items():
        print(f"{cat:<12}{n:>8}{k:>8}")
    print(f"{'合计':<12}{sum(n for n, _ in stats.values()):>8}{total_new:>8}")
    print(f"未收录（字不在词库/音节异常）：{len(dropped)}")

    if write:
        base = os.path.join(WL, "_review.txt")
        with open(base, "w", encoding="utf-8", newline="\n") as f:
            for w, ch, got, want in review:
                f.write(f"{w}\t{ch}\t{got}\t{want}\n")
        with open(os.path.join(WL, "_dropped.txt"), "w", encoding="utf-8", newline="\n") as f:
            for w, why in dropped:
                f.write(f"{w}\t{why}\n")
        print(f"待人工复核 {len(review)} 条 → tools/wordlists/_review.txt")
        print(f"未收录 {len(dropped)} 条 → tools/wordlists/_dropped.txt")
    else:
        print("（只报告，未写文件；加 --write 才写）")


if __name__ == "__main__":
    main()
