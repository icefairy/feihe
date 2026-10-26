#!/usr/bin/env python3
"""生成辅助码表 src/assist.txt

两套方案：

- **自然码辅助码**：取**部首**，1 键。数据源 Unihan `kRSUnicode`（部首序号）
  → 部首读音首字母。例：的→白→`b`、想→心→`x`、打→手→`s`
- **小鹤音形**：取首部件 + 末部件，2 键。数据源是**官方码**，已抓在
  `tools/xhup_cache.txt`（`tools/fetch_xhup.py` 生成），这里只做合并。

小鹤这一列**不再自己推算**。曾经按 IDS 部件分解 + 声母定键算过，实测和官方码
只有 12% 一致（1772 字抽样）——官方有「带上 / 寿上 / 具上 / 既左 / 余下」这类
专用字根，还有「孰大优先 / 相交不拆 / 插隔不取」这些必须靠笔顺几何才能判的规则，
IDS 树推不出来。码错了不会报错，只会让用户筛出空候选，比没有辅助码更糟。

人工修正走 `tools/assist_overrides.txt`（`字<TAB>自然码<TAB>小鹤`，缺的列写 `-`），
优先级最高，改动不会被重跑覆盖。

输出格式（UTF-8，按字排序，缺失用 `-` 占位）：

    字<TAB>自然码辅助码<TAB>小鹤形码

小鹤列可能是多位（`fj|gs`，多音字官方给不同形码），运行时任一位命中即可。

用法：
    python tools/fetch_xhup.py            # 先把官方形码抓齐（增量，只补新字）
    python tools/gen_assist.py            # 再生成
    python tools/gen_assist.py --strict   # 有字缺码就报错退出（发布前用）
"""

import io
import os
import sys
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UNIHAN_ZIP = os.path.join(ROOT, "_unihan.zip")
XHUP_CACHE = os.path.join(ROOT, "tools", "xhup_cache.txt")
OVERRIDES = os.path.join(ROOT, "tools", "assist_overrides.txt")
OUT = os.path.join(ROOT, "src", "assist.txt")
SRC = os.path.join(ROOT, "src")

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
    """返回 字 -> 部首序号"""
    radical = {}
    if not os.path.exists(UNIHAN_ZIP):
        sys.exit(f"缺少 {UNIHAN_ZIP}")
    with zipfile.ZipFile(UNIHAN_ZIP) as z:
        for name in z.namelist():
            if not name.endswith("Unihan_IRGSources.txt"):
                continue
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
    return radical


def dict_chars():
    """词库里出现过的字（现在按首字筛就够，全收着将来按末字筛也用得上）"""
    chars = set()
    for name in sorted(os.listdir(SRC)):
        if not name.endswith(".txt"):
            continue
        if not (name.startswith("dict") or name.startswith("extra")
                or name.startswith("user_dict")):
            continue
        with io.open(os.path.join(SRC, name), encoding="utf-8-sig") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line or line.startswith("#"):
                    continue
                word = line.split("\t")[0].strip()
                if word:
                    chars.update(word)
    return chars


def load_table(path):
    """读 `字<TAB>列1<TAB>列2` 形式的两列/三列表；文件不存在返回空 dict。"""
    table = {}
    if not os.path.exists(path):
        return table
    with io.open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) >= 2 and parts[0].strip():
                table[parts[0].strip()] = [p.strip() for p in parts[1:]]
    return table


def main():
    radical = parse_unihan()
    chars = dict_chars()
    xhup = load_table(XHUP_CACHE)
    over = load_table(OVERRIDES)
    print(f"词库字符数: {len(chars)}   Unihan 部首: {len(radical)}   "
          f"小鹤官方码: {len(xhup)}   人工修正: {len(over)}")

    rows = []
    hit_zrm = hit_xh = 0
    missing_xh = []
    for ch in sorted(chars):
        zrm = RAD_INITIAL[radical[ch] - 1] if ch in radical else "-"
        xh = (xhup.get(ch) or ["-"])[0]
        # 人工修正覆盖一切
        if ch in over:
            cols = over[ch]
            zrm = cols[0] if cols else zrm
            if len(cols) > 1 and cols[1]:
                xh = cols[1]
        if zrm != "-":
            hit_zrm += 1
        if xh and xh != "-":
            hit_xh += 1
        else:
            missing_xh.append(ch)
        if zrm == "-" and (not xh or xh == "-"):
            continue
        rows.append((ch, zrm, xh))

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("# 辅助码表，由 tools/gen_assist.py 生成，勿手改\n")
        f.write("# 字<TAB>自然码辅助码<TAB>小鹤形码（多位用 | 分隔）\n")
        for ch, zrm, xh in rows:
            f.write(f"{ch}\t{zrm}\t{xh}\n")

    size = os.path.getsize(OUT)
    print(f"写出 {OUT}: {len(rows)} 条, {size} bytes")
    print(f"  自然码有码 {hit_zrm}/{len(chars)}   小鹤有码 {hit_xh}/{len(chars)}")
    if missing_xh:
        print(f"  小鹤缺码 {len(missing_xh)} 个：{''.join(missing_xh[:30])}"
              f"{'…' if len(missing_xh) > 30 else ''}")
        if "--strict" in sys.argv:
            sys.exit("小鹤列有缺码，先跑 tools/fetch_xhup.py 补齐（或加 overrides）")

    print("\n抽样：")
    for probe in ("带", "打", "的", "鹤", "形", "一", "想"):
        row = next((r for r in rows if r[0] == probe), None)
        print(f"  {probe}: 自然码={row[1] if row else '缺'}  小鹤={row[2] if row else '缺'}")


if __name__ == "__main__":
    main()
