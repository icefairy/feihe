#!/usr/bin/env python3
"""生成内置精简词库 src/dict.txt

数据源（雾凇拼音 rime-ice，带真实词频）：
  cn_dicts/8105.dict.yaml   单字，约 8700 条，带词频
  cn_dicts/base.dict.yaml   词组，带词频

输出格式（制表符分隔，两列）：
  词<TAB>双拼码

排序规则：双拼码升序，同码内按词频降序。这样运行时只要做前缀匹配，
取到的顺序天然就是高频在前，不需要在 DLL 里再排序，也不用存词频。

用法：
  python tools/gen_dict.py                # 下载（缓存到 _ice*.yaml）并生成
  python tools/gen_dict.py --no-download  # 只用已有缓存
"""

import io
import os
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE_CHAR = os.path.join(ROOT, "_ice8105.yaml")
CACHE_WORD = os.path.join(ROOT, "_icebase.yaml")
OUT = os.path.join(ROOT, "src", "dict.txt")

MIRRORS = [
    "https://ghproxy.net/https://raw.githubusercontent.com/iDvel/rime-ice/main/",
    "https://raw.githubusercontent.com/iDvel/rime-ice/main/",
]

# 与 src/shuangpin.rs 保持完全一致
INITIALS = set("bpmfdtnlgkhjqxrzcsyw")
FINALS = {
    "a": "a", "o": "o", "e": "e", "i": "i", "u": "u", "v": "v",
    "ai": "d", "ei": "w", "ao": "c", "ou": "z",
    "an": "j", "en": "f", "ang": "h", "eng": "g", "ong": "s",
    "ia": "x", "ie": "p", "iao": "n", "iu": "q",
    "ian": "m", "in": "b", "iang": "l", "ing": "k", "iong": "s",
    "ua": "x", "uo": "o", "uai": "k", "ui": "v",
    "uan": "r", "un": "y", "uang": "l",
    "ue": "t", "ve": "t", "van": "r", "vn": "y",
    "er": "r",
}

# 单字：**全收，不按词频截断**。
#
# 早期这里是 3500（按词频截断），埋了个大坑：同事姓「仝」，「仝」在 8105 字表里
# 排 5402 名、词频 796，而第 3500 名的词频下限是 11418 —— 直接被截掉，用户翻页
# 也翻不到，表现成「这个字根本打不出来」。
#
# 生僻字在候选列表里只是尾部多一行（排序在最后，不影响常用字），缺了却是硬伤。
# 姓名、地名、古籍用字都属于这一类，不能靠词频淘汰。
MAX_CHARS = None  # None = 全部收录

# 词组条数。465k 条可编码词组里按词频取前 N 条：
#   6500 → 词频下限 17 万（只有极高频词，覆盖太窄）
#  30000 → 词频下限 3.3 万（约 450KB，常见词基本齐了）
MAX_WORDS = 30000


def encode_syllable(syl):
    s = syl.strip().lower()
    if not s or not s.isascii():
        return None
    # 小鹤声母：zh->v, ch->i, sh->u（与 src/shuangpin.rs 一致）
    if len(s) >= 3 and s[:2] in ("zh", "ch", "sh"):
        ini, fin = {"zh": "v", "ch": "i", "sh": "u"}[s[:2]], s[2:]
    elif s[0] in INITIALS:
        ini, fin = s[0], s[1:]
    else:
        ini, fin = None, s

    key = FINALS.get(fin)
    if key is None:
        return None
    if ini is not None:
        return ini + key
    if s[0] not in ("a", "e", "o"):
        return None
    return s[0] + key


def encode_pinyin(pinyin):
    out = []
    for syl in pinyin.split():
        code = encode_syllable(syl)
        if code is None:
            return None
        out.append(code)
    return "".join(out) if out else None


def normalize_pinyin(pinyin):
    """统一成「小写 + 单空格分隔」的音节串，词库里存这个形式。"""
    return " ".join(pinyin.split()).lower()


def fetch(name, dest):
    for base in MIRRORS:
        url = base + name
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
            data = urllib.request.urlopen(req, timeout=60).read()
            with open(dest, "wb") as f:
                f.write(data)
            print(f"  downloaded {name} ({len(data)} bytes) from {url.split('/')[2]}")
            return True
        except Exception as e:
            print(f"  {url.split('/')[2]} failed: {type(e).__name__}")
    return False


def parse_dict(path):
    """返回 [(text, pinyin, weight)]"""
    lines = io.open(path, encoding="utf-8").read().splitlines()
    start = 0
    for i, l in enumerate(lines):
        if l.strip() == "...":
            start = i + 1
            break
    out = []
    for l in lines[start:]:
        if not l.strip() or l.startswith("#"):
            continue
        parts = l.split("\t")
        if len(parts) < 2:
            continue
        text, pinyin = parts[0].strip(), parts[1].strip()
        weight = 1
        if len(parts) >= 3:
            try:
                weight = int(float(parts[2]))
            except ValueError:
                weight = 1
        out.append((text, pinyin, weight))
    return out


def is_cjk(s):
    return all("\u4e00" <= c <= "\u9fff" for c in s)


def main():
    if "--no-download" not in sys.argv:
        if not os.path.exists(CACHE_CHAR):
            print("fetching char dict...")
            if not fetch("cn_dicts/8105.dict.yaml", CACHE_CHAR):
                sys.exit("cannot download 8105.dict.yaml")
        if not os.path.exists(CACHE_WORD):
            print("fetching word dict...")
            if not fetch("cn_dicts/base.dict.yaml", CACHE_WORD):
                sys.exit("cannot download base.dict.yaml")

    # 词库存「词 + 全拼」，不存双拼码：方案切换/导入导出都不必重新生成数据。
    # 输出按词频降序，DLL 载入时按当前方案编码后做稳定排序，同码内自然保持词频序。
    entries = {}  # (word, pinyin) -> weight

    chars = parse_dict(CACHE_CHAR)
    chars.sort(key=lambda x: -x[2])
    kept = 0
    for text, pinyin, weight in chars:
        if MAX_CHARS is not None and kept >= MAX_CHARS:
            break
        if len(text) != 1 or not is_cjk(text):
            continue
        if encode_pinyin(pinyin) is None:
            continue
        entries[(text, normalize_pinyin(pinyin))] = weight
        kept += 1
    print(f"chars kept: {kept}")

    words = parse_dict(CACHE_WORD) if os.path.exists(CACHE_WORD) else []
    words.sort(key=lambda x: -x[2])
    kept = 0
    for text, pinyin, weight in words:
        if kept >= MAX_WORDS:
            break
        if not (2 <= len(text) <= 4) or not is_cjk(text):
            continue
        if encode_pinyin(pinyin) is None:
            continue
        entries[(text, normalize_pinyin(pinyin))] = weight
        kept += 1
    print(f"words kept: {kept}")

    rows = sorted(entries.items(), key=lambda kv: -kv[1])

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for (word, pinyin), _w in rows:
            f.write(f"{word}\t{pinyin}\n")

    size = os.path.getsize(OUT)
    print(f"wrote {OUT}: {len(rows)} entries, {size} bytes")

    # 自检：按小鹤编码后抽查几个高频码
    probes = ["nihc", "vsgo", "womf", "de", "ui", "vs"]
    for probe in probes:
        hit = [
            w for (w, py), _ in rows
            if (encode_pinyin(py) or "") == probe
        ][:5]
        print(f"  {probe} -> {hit}")


if __name__ == "__main__":
    main()
