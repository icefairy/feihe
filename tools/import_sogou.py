#!/usr/bin/env python3
"""把本机搜狗拼音的用户词库导入飞鹤，生成 src/user_dict.txt。

可反复执行：每次都从搜狗原始文件全量重建，不累积、不残留。

数据源（自动发现）：
  <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usrbg.bin   用户背景词库（打字历史）
  <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usr.bin     用户词库
  <搜狗安装目录>/Scd/*.scel                                      细胞词库
  <用户数据目录>/SogouPY/users/<id>/UserDictionary.txt            手动导出的文本词库

注音策略（重要）：
  1. 先在基础词库 src/dict.txt 里查——那里已经是「词 → 小鹤双拼码」，多音字天然正确
  2. 查不到才逐字注音（单字取 8105 字典里词频最高的读音），再用小鹤表编码

输出格式与 dict.txt 一致：`词<TAB>双拼码`，按「码升序、同码内优先级降序」预排。
dict.rs 会先加载 user_dict.txt 再加载 dict.txt，同码时用户词排前面。

用法：
  python tools/import_sogou.py                 # 自动发现并重建
  python tools/import_sogou.py --dir <路径>     # 手动指定搜狗用户数据目录
  python tools/import_sogou.py --list          # 只列出发现的文件
  python tools/import_sogou.py --dry-run       # 只统计不写文件
  python tools/import_sogou.py --max-len 8     # 允许更长的短语（默认 6）
"""

import io
import os
import re
import struct
import sys
from collections import Counter, defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_dict as gd  # 复用同一张双拼表，避免两处不一致

ROOT = gd.ROOT
OUT = os.path.join(ROOT, "src", "user_dict.txt")
BASE_DICT = os.path.join(ROOT, "src", "dict.txt")

CJK_MIN, CJK_MAX = 0x4E00, 0x9FFF

DEFAULT_MAX_LEN = 6


# ─────────────────────────────────────────────────────────────
# 发现搜狗数据
# ─────────────────────────────────────────────────────────────


def read_install_path():
    """从注册表读搜狗安装目录（32 位视图）。"""
    try:
        import winreg
    except ImportError:
        return None
    keys = [
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\SogouInput"),
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\SogouInput"),
        (winreg.HKEY_CURRENT_USER, r"SOFTWARE\SogouInput"),
    ]
    for hive, path in keys:
        try:
            with winreg.OpenKey(hive, path) as k:
                val, _ = winreg.QueryValueEx(k, "")
                if val and os.path.isdir(val):
                    return val
        except OSError:
            continue
    return None


def candidate_roots():
    """可能存放搜狗用户数据的目录。"""
    roots = []
    install = read_install_path()
    if install:
        roots.append(("注册表安装目录", install))
    for env in ("PROGRAMFILES", "PROGRAMFILES(X86)"):
        p = os.environ.get(env)
        if p:
            roots.append((env, os.path.join(p, "SogouInput")))
    appdata = os.environ.get("APPDATA")
    localappdata = os.environ.get("LOCALAPPDATA")
    if appdata:
        roots.append(("APPDATA", os.path.join(appdata, "SogouPY")))
        roots.append(("APPDATA", os.path.join(appdata, "SogouInput")))
    if localappdata:
        roots.append(("LOCALAPPDATA", os.path.join(localappdata, "SogouPY")))
    seen = set()
    out = []
    for label, p in roots:
        if p and p not in seen and os.path.isdir(p):
            seen.add(p)
            out.append((label, p))
    return out


def find_user_data_dirs(roots):
    """找出所有含 sgim_*.bin 的用户数据目录。"""
    dirs = []
    for label, root in roots:
        # <root>/用户数据/SogouPY.users/<id>/
        for rel in ("用户数据/SogouPY.users", "users", "SogouPY.users"):
            base = os.path.join(root, *rel.split("/"))
            if not os.path.isdir(base):
                continue
            for name in os.listdir(base):
                sub = os.path.join(base, name)
                if os.path.isdir(sub) and any(
                    f.startswith("sgim_") for f in os.listdir(sub)
                ):
                    dirs.append((f"{label}:{rel}/{name}", sub))
    return dirs


def find_scel_dirs(roots):
    out = []
    for label, root in roots:
        scd = os.path.join(root, "Scd")
        if os.path.isdir(scd):
            out.append((f"{label}:Scd", scd))
    return out


# ─────────────────────────────────────────────────────────────
# 解析各种词库文件
# ─────────────────────────────────────────────────────────────


def _all_cjk(s):
    return all(CJK_MIN <= ord(c) <= CJK_MAX for c in s)


def extract_bin_strict(path):
    """sgim_gd_usrbg.bin 的记录格式：
        [u16 字节长][UTF-16LE 文本][u16 字节长+6]
    最后那个字段等于「字节长 + 6」，拿来当校验和，能把字节错位的假词全部滤掉。
    """
    data = open(path, "rb").read()
    out = []
    n = len(data)
    for p in range(0, n - 6, 2):
        (blen,) = struct.unpack_from("<H", data, p)
        if blen < 4 or blen > 32 or blen % 2:
            continue
        end = p + 2 + blen
        if end + 2 > n:
            continue
        (tail,) = struct.unpack_from("<H", data, end)
        if tail != blen + 6:
            continue
        try:
            text = data[p + 2 : end].decode("utf-16-le")
        except UnicodeDecodeError:
            continue
        if _all_cjk(text):
            out.append(text)
    return out


def extract_bin_loose(path):
    """sgim_gd_usr.bin 的记录格式与 usrbg 不同（尾部校验值不是 len+6），
    退回宽松规则：只要 [u16 偶数长度][全 CJK 文本]。
    噪声靠后面的「必须能注音」过滤掉。"""
    data = open(path, "rb").read()
    out = []
    n = len(data)
    for p in range(0, n - 4, 2):
        (blen,) = struct.unpack_from("<H", data, p)
        if blen < 4 or blen > 32 or blen % 2:
            continue
        end = p + 2 + blen
        if end > n:
            continue
        try:
            text = data[p + 2 : end].decode("utf-16-le")
        except UnicodeDecodeError:
            continue
        if _all_cjk(text):
            out.append(text)
    return out


def extract_scel(path):
    """搜狗细胞词库 .scel。格式：
        "SougouPY" + 0x0A + 4 个以 0x0A00 结尾的 UTF-16LE 字段 + 词条区
    词条区： [u16 拼音字符数][拼音 UTF-16LE][u16 词字符数][词 UTF-16LE]
    各版本头部字段数略有差异，这里从 4 个候选起点里挑解析成功最多的那个。
    """
    data = open(path, "rb").read()
    if not data.startswith(b"SougouPY"):
        return []
    pos = 9
    for _ in range(4):
        while pos + 1 < len(data) and data[pos : pos + 2] != b"\x0a\x00":
            pos += 2
        pos += 2
    if pos >= len(data):
        return []

    def parse(start):
        out = []
        p = start
        while p + 4 <= len(data):
            (plen,) = struct.unpack_from("<H", data, p)
            if plen == 0 or plen > 200:
                break
            p += 2
            if p + plen * 2 + 2 > len(data):
                break
            p += plen * 2
            (wlen,) = struct.unpack_from("<H", data, p)
            if wlen == 0 or wlen > 64:
                break
            p += 2
            if p + wlen * 2 > len(data):
                break
            try:
                word = data[p : p + wlen * 2].decode("utf-16-le")
            except UnicodeDecodeError:
                break
            p += wlen * 2
            if _all_cjk(word):
                out.append(word)
        return out

    best = []
    for start in (pos, pos + 4, pos + 2, pos + 8):
        got = parse(start)
        if len(got) > len(best):
            best = got
    return best


def extract_txt(path):
    """搜狗「导出词库」或用户自备的文本：每行 `词` 或 `词<TAB>拼音` 或 `词 拼音`。"""
    out = []
    try:
        text = io.open(path, encoding="utf-8", errors="ignore").read()
    except OSError:
        return out
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        word = re.split(r"[\t ]", line)[0].strip()
        if word and _all_cjk(word):
            out.append(word)
    return out


# ─────────────────────────────────────────────────────────────
# 注音
# ─────────────────────────────────────────────────────────────


def load_base_codes():
    """基础词库是「词 → 全拼」，直接拿来用，多音字天然正确。"""
    codes = {}
    if not os.path.exists(BASE_DICT):
        return codes
    with io.open(BASE_DICT, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 2:
                continue
            word, pinyin = parts
            codes.setdefault(word, pinyin)
    return codes


def load_char_pinyin():
    """单字 → 最高频读音。数据来自 8105 字典（带真实词频）。"""
    if not os.path.exists(gd.CACHE_CHAR):
        print("  ! 找不到 _ice8105.yaml，无法逐字注音（先跑一次 gen_dict.py）")
        return {}
    best = {}
    for text, pinyin, weight in gd.parse_dict(gd.CACHE_CHAR):
        if len(text) != 1 or not _all_cjk(text):
            continue
        syl = pinyin.strip()
        if not syl or " " in syl:
            continue
        if text not in best or weight > best[text][1]:
            best[text] = (syl, weight)
    return {k: v[0] for k, v in best.items()}


class Encoder:
    def __init__(self):
        self.base_codes = load_base_codes()
        self.char_pinyin = load_char_pinyin()
        self.from_base = 0
        self.from_char = 0
        self.unencodable = 0

    def encode(self, word):
        pinyin = self.base_codes.get(word)
        if pinyin:
            self.from_base += 1
            return pinyin
        syls = []
        for ch in word:
            py = self.char_pinyin.get(ch)
            if not py:
                self.unencodable += 1
                return None
            syls.append(py)
        pinyin = " ".join(syls)
        # 校验：至少小鹤方案能编码，否则这词没法用
        code = gd.encode_pinyin(pinyin)
        if code is None or len(code) != 2 * len(word):
            self.unencodable += 1
            return None
        self.from_char += 1
        return pinyin


# ─────────────────────────────────────────────────────────────
# 主流程
# ─────────────────────────────────────────────────────────────


def collect(sources, max_len):
    """sources: [(label, words, priority)] -> {word: (priority, count)}"""
    agg = {}
    for label, words, prio in sources:
        kept = 0
        for w in words:
            if not (2 <= len(w) <= max_len):
                continue
            prev = agg.get(w)
            if prev is None:
                agg[w] = (prio, 1)
                kept += 1
            else:
                agg[w] = (max(prev[0], prio), prev[1] + 1)
        print(f"  {label:<28} {len(words):>7} 条 → 收 {kept:>6}")
    return agg


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass

    args = sys.argv[1:]
    dry_run = "--dry-run" in args
    list_only = "--list" in args

    max_len = DEFAULT_MAX_LEN
    if "--max-len" in args:
        max_len = int(args[args.index("--max-len") + 1])

    manual_dir = None
    if "--dir" in args:
        manual_dir = args[args.index("--dir") + 1]

    # --- 发现 ---
    if manual_dir:
        roots = [("手动指定", manual_dir)]
        user_dirs = [("手动指定", manual_dir)]
        scel_dirs = [("手动指定", manual_dir)]
    else:
        roots = candidate_roots()
        if not roots:
            sys.exit(
                "找不到搜狗输入法。请用 --dir 手动指定用户数据目录，例如：\n"
                r'  python tools/import_sogou.py --dir "D:\soft\sogoupinyin\用户数据\SogouPY.users\00000001"'
            )
        user_dirs = find_user_data_dirs(roots)
        scel_dirs = find_scel_dirs(roots)

    print("搜狗数据目录：")
    for label, p in roots:
        print(f"  [{label}] {p}")
    print()

    # --- 收集文件 ---
    jobs = []  # (label, path, extractor, priority)
    for label, d in user_dirs:
        bg = os.path.join(d, "sgim_gd_usrbg.bin")
        usr = os.path.join(d, "sgim_gd_usr.bin")
        if os.path.exists(bg):
            jobs.append((f"{label}/usrbg", bg, extract_bin_strict, 1))
        if os.path.exists(usr):
            jobs.append((label + "/usr", usr, extract_bin_loose, 2))
        for name in os.listdir(d):
            low = name.lower()
            if low.endswith(".txt") and ("dict" in low or "词库" in name):
                jobs.append((f"{label}/{name}", os.path.join(d, name), extract_txt, 3))
    for label, d in scel_dirs:
        for name in os.listdir(d):
            if name.lower().endswith(".scel"):
                jobs.append((f"{label}/{name}", os.path.join(d, name), extract_scel, 3))

    if not jobs:
        sys.exit("没有发现任何可解析的词库文件。")

    print("词库文件：")
    for label, path, fn, prio in jobs:
        size = os.path.getsize(path)
        print(f"  [{prio}] {label}  ({size:,} bytes)")
    print()

    if list_only:
        return

    # --- 提取 ---
    print("提取：")
    sources = []
    for label, path, fn, prio in jobs:
        try:
            words = fn(path)
        except Exception as e:
            print(f"  {label:<28} 解析失败: {type(e).__name__}: {e}")
            continue
        sources.append((label, words, prio))
    print()

    agg = collect(sources, max_len)
    print(f"\n去重后候选词：{len(agg)}")

    # --- 注音 ---
    enc = Encoder()
    rows = []
    for word, (prio, count) in agg.items():
        pinyin = enc.encode(word)
        if pinyin:
            rows.append((word, pinyin, prio, count))
    print(f"可注音：{len(rows)}  （基础词库命中 {enc.from_base}，逐字注音 {enc.from_char}，失败 {enc.unencodable}）")

    # --- 排序：优先级高、出现次数多、词短的在前（与方案无关，方案由 DLL 侧编码）---
    rows.sort(key=lambda r: (-r[2], -r[3], len(r[0])))

    # --- 去重（同词只留一条）---
    seen = set()
    final = []
    for word, pinyin, prio, count in rows:
        if word in seen:
            continue
        seen.add(word)
        final.append((word, pinyin))

    print(f"最终写入：{len(final)} 条")

    if dry_run:
        print("\n--dry-run，不写文件。前 30 条预览：")
        for w, p in final[:30]:
            print(f"  {w}\t{p}")
        return

    # --- 写出 ---
    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for word, pinyin in final:
            f.write(f"{word}\t{pinyin}\n")

    size = os.path.getsize(OUT)
    print(f"已写入 {OUT}（{size:,} bytes）")

    # --- 抽样自检（按小鹤方案临时编码后比对）---
    print("\n抽样（小鹤码）：")
    for probe in ("nihc", "vsgo", "womf", "uijm"):
        hit = [
            w for w, p in final
            if (gd.encode_pinyin(p) or "") == probe
        ][:6]
        if hit:
            print(f"  {probe} -> {hit}")
    print("\n重新构建 DLL 后生效：build.ps1 → 替换 C:\\ProgramData\\Feihe\\feihe.dll")


if __name__ == "__main__":
    main()
