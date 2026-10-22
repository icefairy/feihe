#!/usr/bin/env python3
"""生成辅助码表 src/assist.txt

两套方案，都是「部件 → 声母定键」，区别是取几个部件：

- 自然码辅助码：取**部首**，1 键。数据源 Unihan `kRSUnicode`（部首序号）
- 小鹤音形：取**首部件 + 末部件**，2 键。数据源 cjkvi-ids 的 IDS 分解

字根键位按小鹤官方规则「多数采用声母定键，即字根的声母就是键位」
（见 形 = 双拼 xk + 首形 开(k) + 末形 撇(p) = xkkp）。

输出格式（UTF-8，按字排序）：

    字<TAB>自然码辅助码<TAB>小鹤形码

缺失的用 `-` 占位。只输出我们词库里出现过的字，文件很小。

用法：
    python tools/gen_assist.py                # 全量（需要 _unihan.zip 和 _ids.txt）
    python tools/gen_assist.py --no-ids       # 只生成自然码那列
"""

import io
import os
import sys
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UNIHAN_ZIP = os.path.join(ROOT, "_unihan.zip")
IDS_TXT = os.path.join(ROOT, "_ids.txt")
OUT = os.path.join(ROOT, "src", "assist.txt")
DICTS = [os.path.join(ROOT, "src", "dict.txt"), os.path.join(ROOT, "src", "user_dict.txt")]

# 康熙部首 1..214 的拼音首字母，按部首序号排列，每行 10 个便于核对。
#   1 一(y)      2 丨(gǔn)   3 丶(zhǔ)   4 丿(piě)   5 乙(y)    6 亅(jué)
#   7 二(e)      8 亠(tóu)   9 人(r)    10 儿(e)    11 入(r)   12 八(b)
#  13 冂(jiōng) 14 冖(mì)   15 冫(bīng) 16 几(j)    17 凵(kǎn) 18 刀(d)
#  27 厂(chǎng) 40 宀(mián) 54 廴(yǐn)  55 廾(gǒng) 58 彐(jì)  59 彡(shān)
#  60 彳(chì)   66 攴(pū)   79 殳(shū)  90 爿(pán) 103 疋(pǐ) 104 疒(nè)
# 105 癶(bō)   114 禸(róu) 120 糸(sī)  121 缶(fǒu) 127 耒(lěi) 136 舛(chuǎn)
# 140 艸(cǎo)  141 虍(hū)  152 豕(shǐ) 153 豸(zhì) 162 辵(chuò) 165 釆(biàn)
# 171 隶(lì)   172 隹(zhuī) 190 髟(biāo) 192 鬯(chàng) 193 鬲(lì) 204 黹(zhǐ)
# 205 黽(mǐn)  214 龠(yuè)
RAD_INITIAL = (
    # 001-010
    "y g z p y j e t r e "
    # 011-020
    "r b j m b j k d l b "
    # 021-030
    "b f x s b j c s y k "
    # 031-040
    "w t s z s x d n z m "
    # 041-050
    "c x w s c s c g j j "
    # 051-060
    "g y g y g y g j s c "
    # 061-070
    "x g h s z p w d j f "
    # 071-080
    "w r y y m q z d s w "
    # 081-090
    "b m s q s h z f y p "
    # 091-100
    "p y n q x y g w g s "
    # 101-110
    "y t p n b b p m m m "
    # 111-120
    "s s s r h x l z m s "
    # 121-130
    "f w y y l e l e y r "
    # 131-140
    "c z z j s c z g s c "
    # 141-150
    "h c x x y y j j y g "
    # 151-160
    "d s z b c z z s c x "
    # 161-170
    "c c y y b l j c m f "
    # 171-180
    "l z y q f m g w j y "
    # 181-190
    "y f f s s x m g g b "
    # 191-200
    "d c l g y n l l m m "
    # 201-210
    "h s h z m d g s b q "
    # 211-214
    "c l g y"
).split()

assert len(RAD_INITIAL) == 214, f"部首表应有 214 项，实际 {len(RAD_INITIAL)}"


def parse_unihan():
    """返回 (字 -> 部首序号, 字 -> 拼音首字母)"""
    radical = {}
    mandarin = {}
    if not os.path.exists(UNIHAN_ZIP):
        sys.exit(f"缺少 {UNIHAN_ZIP}")
    with zipfile.ZipFile(UNIHAN_ZIP) as z:
        for name in z.namelist():
            if name.endswith("Unihan_IRGSources.txt"):
                for line in io.TextIOWrapper(z.open(name), encoding="utf-8"):
                    if line.startswith("#"):
                        continue
                    parts = line.rstrip("\n").split("\t")
                    if len(parts) < 3 or parts[1] != "kRSUnicode":
                        continue
                    cp = int(parts[0][2:], 16)
                    # 值形如 "9.0" 或 "9.0'"，可能多组用空格分隔；带 ' 的是简化部首形
                    num = parts[2].split()[0].split(".")[0].rstrip("'")
                    try:
                        n = int(num)
                    except ValueError:
                        continue
                    if 1 <= n <= 214:
                        radical.setdefault(chr(cp), n)
            elif name.endswith("Unihan_Readings.txt"):
                for line in io.TextIOWrapper(z.open(name), encoding="utf-8"):
                    if line.startswith("#"):
                        continue
                    parts = line.rstrip("\n").split("\t")
                    if len(parts) < 3 or parts[1] != "kMandarin":
                        continue
                    cp = int(parts[0][2:], 16)
                    py = parts[2].split()[0]
                    # 去掉声调符号，取首字母
                    letters = "".join(c for c in py if c.isascii() and c.isalpha())
                    if letters:
                        mandarin.setdefault(chr(cp), letters[0].lower())
    return radical, mandarin


# IDS 运算符 → 参与拆分的子部件个数
IDS_OPS = {
    "\u2ff0": 2, "\u2ff1": 2, "\u2ff2": 3, "\u2ff3": 3,
    "\u2ff4": 2, "\u2ff5": 2, "\u2ff6": 2, "\u2ff7": 2,
    "\u2ff8": 2, "\u2ff9": 2, "\u2ffa": 2, "\u2ffb": 2,
}


def parse_one(s):
    """解析一个完整 IDS 表达式，返回它消耗的字符数。"""
    if not s:
        return 0
    n = IDS_OPS.get(s[0])
    if not n:
        return 1
    i, cnt = 1, 0
    while i < len(s) and cnt < n:
        i += parse_one(s[i:])
        cnt += 1
    return i


def split_top(s):
    """把 IDS 表达式拆成顶层部件列表；不是 IDS 就返回单元素。"""
    if not s:
        return []
    n = IDS_OPS.get(s[0])
    if not n:
        return [s]
    parts, i = [], 1
    while i < len(s) and len(parts) < n:
        used = parse_one(s[i:])
        parts.append(s[i:i + used])
        i += used
    return parts


def first_leaf(s):
    """最左侧的叶子部件。"""
    if not s:
        return None
    if s[0] in IDS_OPS:
        return first_leaf(s[1:])
    return s[0]


def last_leaf(s):
    """最右侧的叶子部件。"""
    parts = split_top(s)
    if not parts:
        return None
    return last_leaf(parts[-1]) if parts[-1][0] in IDS_OPS else parts[-1][0]


def parse_ids():
    """返回 字 -> 顶层部件列表"""
    table = {}
    if not os.path.exists(IDS_TXT):
        return table
    with io.open(IDS_TXT, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#"):
                continue
            parts = line.rstrip("\n").split("\t")
            if len(parts) < 2:
                continue
            try:
                cp = int(parts[0][2:], 16)
            except ValueError:
                continue
            # 格式：U+XXXX<TAB>字<TAB>IDS
            ids = (parts[2] if len(parts) >= 3 else parts[1]).strip()
            if not ids or ids == parts[1].strip():
                continue
            table.setdefault(chr(cp), ids)
    return table


def load_dict_chars():
    chars = set()
    for path in DICTS:
        if not os.path.exists(path):
            continue
        with io.open(path, encoding="utf-8") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line or line.startswith("#"):
                    continue
                word = line.split("\t")[0]
                chars.update(word)
    return chars


def main():
    radical, mandarin = parse_unihan()
    ids = {} if "--no-ids" in sys.argv else parse_ids()
    chars = load_dict_chars()
    print(f"词库字符数: {len(chars)}")
    print(f"Unihan 部首覆盖: {len(radical)}   拼音覆盖: {len(mandarin)}   IDS 覆盖: {len(ids)}")

    rows = []
    hit_zrm = hit_xh = 0
    for ch in sorted(chars):
        # 自然码：部首声母
        zrm = "-"
        n = radical.get(ch)
        if n:
            zrm = RAD_INITIAL[n - 1]
            hit_zrm += 1

        # 小鹤音形：首部件 + 末部件，声母定键
        xh = "-"
        expr = ids.get(ch, "")
        if expr:
            a, b = first_leaf(expr), last_leaf(expr)
            ka = mandarin.get(a) if a else None
            kb = mandarin.get(b) if b else None
            if ka and kb:
                xh = ka + kb
                hit_xh += 1

        if zrm == "-" and xh == "-":
            continue
        rows.append((ch, zrm, xh))

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("# 辅助码表，由 tools/gen_assist.py 生成，勿手改\n")
        f.write("# 字<TAB>自然码辅助码<TAB>小鹤形码\n")
        for ch, zrm, xh in rows:
            f.write(f"{ch}\t{zrm}\t{xh}\n")

    size = os.path.getsize(OUT)
    print(f"写出 {OUT}: {len(rows)} 条, {size} bytes")
    print(f"  自然码有码: {hit_zrm}   小鹤形码: {hit_xh}")
    print("\n抽样：")
    for probe in ("的", "你", "好", "是", "中", "形", "鹤", "想", "打"):
        row = next((r for r in rows if r[0] == probe), None)
        print(f"  {probe}: 自然码={row[1] if row else '缺'}  小鹤={row[2] if row else '缺'}")


if __name__ == "__main__":
    main()
