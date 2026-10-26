#!/usr/bin/env python3
"""从小鹤官方接口抓「小鹤音形」形码，落地到 tools/xhup_cache.txt

为什么需要它：`gen_assist.py` 原来的小鹤列是按 IDS 部件分解 + 声母定键**推算**的，
和官方码表在不少字上对不上（官方的「带」是 ddfj，推算出来是 sj）。用户按官方反查
工具的码去敲辅助码，一个字都筛不出来——这比没有辅助码更糟，因为它看起来像坏了。

官方接口（react.xhup.club 前端调的就是这个）：

    POST http://www.xhup.club/Xhup/Search/searchCode
    search_word=<字>&sign=md5("fjc_xhup" + 字)

返回的 `list_dz[i]` 是

    [展示串, 部件分解, 首部件, 末部件, 首形键, 末形键, 字]

其中 `list_dz[i][4] + list_dz[i][5]` 就是形码（不含双拼那两位）。多音字会返回多条，
形码通常一致；真不一致时存成 `a|b` 让运行时任一命中即可。

输出 `tools/xhup_cache.txt`（按字排序，`-` 表示官方查不到），**可提交、增量更新**：
已经缓存过的字不再请求，以后词库加字只需重跑一次。

用法：
    python tools/fetch_xhup.py             # 补齐缺的字（并发 6，约 280ms/字）
    python tools/fetch_xhup.py --refetch   # 全部重抓（官方改码表时用）
"""

import hashlib
import io
import json
import os
import ssl
import sys
import time
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.path.join(ROOT, "tools", "xhup_cache.txt")
SRC = os.path.join(ROOT, "src")

API = "http://www.xhup.club/Xhup/Search/searchCode"
SALT = "fjc_xhup"
WORKERS = 6
RETRY = 3

ssl._create_default_https_context = ssl._create_unverified_context


def wanted_chars():
    """词库里出现过、且会当「词首」的字——辅助码筛选只按首字算。"""
    chars = set()
    names = sorted(n for n in os.listdir(SRC) if n.endswith(".txt"))
    for n in names:
        if not (n.startswith("dict") or n.startswith("extra") or n.startswith("user_dict")):
            continue
        path = os.path.join(SRC, n)
        with io.open(path, encoding="utf-8-sig") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line or line.startswith("#"):
                    continue
                word = line.split("\t")[0].strip()
                if word:
                    chars.add(word[0])
    return chars


def load_cache():
    table = {}
    if os.path.exists(CACHE):
        with io.open(CACHE, encoding="utf-8") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line or line.startswith("#"):
                    continue
                parts = line.split("\t")
                if len(parts) >= 2:
                    table[parts[0]] = parts[1]
    return table


def save_cache(table):
    with io.open(CACHE, "w", encoding="utf-8", newline="\n") as f:
        f.write("# 小鹤音形形码，tools/fetch_xhup.py 从官方接口抓取，勿手改\n")
        f.write("# 字<TAB>形码（多位读音不一致时用 | 分隔）；- 表示官方无结果\n")
        for ch in sorted(table):
            f.write(f"{ch}\t{table[ch]}\n")


def query(ch):
    sign = hashlib.md5((SALT + ch).encode()).hexdigest()
    body = urllib.parse.urlencode({"search_word": ch, "sign": sign}).encode()
    req = urllib.request.Request(
        API,
        data=body,
        headers={
            "Content-Type": "application/x-www-form-urlencoded",
            "Origin": "http://react.xhup.club",
            "Referer": "http://react.xhup.club/",
            "User-Agent": "Mozilla/5.0",
        },
    )
    raw = urllib.request.urlopen(req, timeout=20).read().decode("utf-8")
    return json.loads(raw)


def codes_of(reply):
    """从返回里取形码集合。官方把「首形键 + 末形键」摊在 [4] [5] 两个位置。"""
    out = []
    if reply.get("msg") != "success":
        return out
    for row in reply.get("list_dz") or []:
        if len(row) < 6:
            continue
        code = (row[4] + row[5]).strip().lower()
        code = "".join(c for c in code if c.isascii() and c.isalpha())
        if code and code not in out:
            out.append(code)
    return out


def fetch_one(ch):
    last = None
    for i in range(RETRY):
        try:
            got = codes_of(query(ch))
            if got:
                return ch, "|".join(got[:2])
            return ch, "-"
        except Exception as e:  # 网络抖动 / 接口抽风：退避重试，别把 - 写进缓存
            last = e
            time.sleep(1.0 * (i + 1))
    print(f"  放弃 {ch}: {last}", file=sys.stderr)
    return ch, None


def main():
    chars = wanted_chars()
    cache = {} if "--refetch" in sys.argv else load_cache()
    todo = sorted(chars - set(cache))
    print(f"需要 {len(chars)} 字，已缓存 {len(chars) - len(todo)}，本次抓 {len(todo)}")
    if not todo:
        return

    t0 = time.time()
    done = 0
    failed = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        for ch, code in pool.map(fetch_one, todo):
            if code is None:
                failed += 1
                continue
            cache[ch] = code
            done += 1
            if done % 200 == 0:
                save_cache(cache)  # 中途落盘，跑一半断了不用从头再来
                print(f"  {done}/{len(todo)}  {time.time() - t0:.0f}s")

    save_cache(cache)
    got = sum(1 for c in chars if cache.get(c, "-") != "-")
    print(f"完成：写 {CACHE}")
    print(f"  覆盖 {got}/{len(chars)} 字，有码率 {got * 100 // len(chars)}%，"
          f"失败 {failed}，耗时 {time.time() - t0:.0f}s")


if __name__ == "__main__":
    main()
