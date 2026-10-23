#!/usr/bin/env python
"""生成 `src/hint.txt`：易错词的带调读音 + 错误读音容错码。

## 这张表解决两个问题

1. **看不见读音**：`重庆` 到底读 chóng 还是 zhòng、`水雷屯` 是 zhūn 还是 tún，
   词库里只有不带调的全拼（`chong qing`），用户看不出区别。
2. **按错音打不出来**：多音字词只挂了一个读音，打 `zhongqing` 出的是「钟情」，
   「重庆」永远不出现——用户并不知道自己读错了，只会以为输入法没这个词。

## 「易错」怎么判定

对每个词，逐字比较：

- `词库读音`：`src/dict*.txt` / `src/extra_*.txt` 里那个词条的全拼（人核对过的，
  包括 `tools/pinyin_overrides.txt` 钉的古读）；
- `字级默认读音`：pypinyin 对这个**单字**的首选读音（`弹`→dan、`重`→zhong）。

**某个字的词库读音 ≠ 该字的默认读音，且该字是多音字**，这个词就算易错。
例：`弹出` = tan chu，而 `弹` 默认 dàn → 易错；`重审` = zhong shen，正好等于
默认读音 → 不算易错（虽然有人会念错，但默认读音本来就是对的，不需要提示）。

## 声调从哪来

词库全拼只到音节，声调必须逐词确定（`发` 在「头发」读 fà、在「发现」读 fā，
音节相同）。做法：拿该字的**全部**带调读音（`pinyin(ch, heteronym=True, TONE3)`），
只保留「去调后等于词库音节」的那些，再用 pypinyin 的词级判断在候选里挑一个。
音节对不上就直接报错退出——宁可没有提示，也不能显示错的读音。

## 第三列（错误读音）

「按字级默认读音拼出来的全拼」。运行时把它按当前双拼方案编码，前缀命中就
把词补进候选末尾，于是 `zhongqing` 也能出「重庆」。

只在**去调后与正确读音不同**时才写这一列：只差声调的情况（`着力` zhuó/zhe）
无声调索引本来就相同，打出来已经能出，不需要容错。

## 用法

    python tools/gen_hint.py            # 只报告
    python tools/gen_hint.py --write    # 写 src/hint.txt
"""

import glob
import os
import sys

from pypinyin import Style, lazy_pinyin, pinyin
from pypinyin.contrib.tone_convert import to_normal, to_tone

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "src", "hint.txt")


def strip_tone(s: str) -> str:
    """带调拼音 → 无声调（`chóng` → `chong`）。走 pypinyin 的数字调中转，
    比维护一张韵母声调表可靠。"""
    out = []
    for syl in s.split():
        t3 = _to_tone3(syl)
        out.append(to_normal(t3) if t3 else syl)
    return " ".join(out)


def _to_tone3(syl: str) -> str:
    try:
        from pypinyin.contrib.tone_convert import to_tone3

        return to_tone3(syl)
    except Exception:
        return ""


def load_words():
    """词 → 词库全拼。先到先得（和 `dictenc::push_entries` 的去重语义一致）。"""
    files = ["src/dict.txt"]
    files += sorted(glob.glob(os.path.join(ROOT, "src", "extra_*.txt")))
    files.append(os.path.join(ROOT, "src", "extra_dict.txt"))

    seen, out = set(), []
    for rel in files:
        path = rel if os.path.isabs(rel) else os.path.join(ROOT, rel)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8-sig") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                w, _, py = line.partition("\t")
                w = w.strip()
                if not w or w in seen:
                    continue
                seen.add(w)
                out.append((w, py.strip() or " ".join(lazy_pinyin(w))))
    return out


def is_polyphone(ch: str) -> bool:
    return len(set(pinyin(ch, heteronym=True, style=Style.TONE3)[0])) > 1


def toned_reading(word: str, base: str):
    """按词库音节给每个字挑带调读音。返回 (list[str] | None, 失败说明)。"""
    target = base.split()
    if len(target) != len(word):
        return None, f"音节数 {len(target)} != 字数 {len(word)}"

    pref = [c[0] for c in pinyin(word, style=Style.TONE3)]
    out = []
    for i, ch in enumerate(word):
        cands = pinyin(ch, heteronym=True, style=Style.TONE3)[0]
        match = [c for c in cands if to_normal(c) == target[i]]
        if not match:
            return None, f"「{ch}」没有 {target[i]} 这个读音"
        pick = pref[i] if i < len(pref) and pref[i] in match else match[0]
        out.append(to_tone(pick))
    return out, None


def main():
    write = "--write" in sys.argv
    words = load_words()

    rows, bad = [], []
    for w, base in words:
        # 单字不提示：打单字时用户已经看见字形了，读音提示只会占地方
        if len(w) < 2:
            continue
        if not any(is_polyphone(ch) for ch in w):
            continue
        target = base.split()
        if len(target) != len(w):
            continue
        default = [(lazy_pinyin(ch) or [""])[0] for ch in w]
        if all(target[i] == default[i] for i in range(len(w))):
            continue  # 读音和字级默认一致 → 不易错

        toned, err = toned_reading(w, base)
        if err:
            bad.append((w, base, err))
            continue
        toned_s = " ".join(toned)

        # 错音全拼：按字级默认读音拼。去调后相同就不写（打对了本来就能出）
        wrong = "".join(default)
        right_plain = "".join(target)
        row = f"{w}\t{toned_s}"
        if wrong != right_plain:
            row += "\t" + " ".join(default)
        rows.append(row)

    rows.sort()
    print(f"词条 {len(words)}，易错词 {len(rows)}，失败 {len(bad)}")
    if bad:
        print("!! 有词算不出声调，说明词库读音和 pypinyin 对不上，必须人工看：")
        for w, base, err in bad[:20]:
            print("   ", w, base, err)
        return 1

    head = (
        "# 易错词读音表（自动生成，勿手改）\n"
        "# 由 tools/gen_hint.py 从词库 + pypinyin 生成，重跑：python tools/gen_hint.py --write\n"
        "# 格式：词<TAB>带调读音[<TAB>按字级默认读音拼出的全拼]\n"
        "# 第三列 = 「用户可能按错的读音」，运行时编码后作为容错候选，追加在候选末尾。\n"
    )
    text = head + "\n".join(rows) + "\n"
    print(f"表大小 {len(text.encode())} 字节 / {len(rows)} 条")
    if write:
        with open(OUT, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print("已写", OUT)
    return 0


if __name__ == "__main__":
    sys.exit(main())
