#!/usr/bin/env python3
"""拉取内置的离线 ASR 模型（paraformer-zh-small，79 MB）。

## 为什么要脚本，而不是把 onnx 放进仓库

79 MB 的二进制放 git 里，每次 clone 都要背一遍，历史里还删不掉。模型是**构建输入**，
不是源码——用脚本拉、用 SHA256 钉，和 `tools/gen_dict.py` 那类生成器同一个思路。

## 为什么走 hf-mirror

- `github.com` 在这台机器上**完全不通**（实测 http=000），官方 release 下不了；
- ModelScope 上**没有** `csukuangfj/sherpa-onnx-paraformer-zh-small-2024-03-09`
  这个仓库（`/repo/files` 返回 record not found）；
- `hf-mirror.com` 实测 **663 KB/s**、支持 Range（HTTP 206），79 MB 约 2 分钟。

## 用法

    python tools/fetch_asr_model.py            # 下到 android/app/src/main/assets/asr/
    python tools/fetch_asr_model.py --verify   # 只校验，不下载

下载完成后把打印出来的 SHA256 填进下面的 `EXPECTED`，以后 `--verify` 就能挡住
「上游换了文件」这种事故。
"""

import argparse
import hashlib
import os
import sys
import urllib.request

REPO = "csukuangfj/sherpa-onnx-paraformer-zh-small-2024-03-09"
MIRROR = "https://hf-mirror.com"
FILES = ["model.int8.onnx", "tokens.txt"]

# 文件名 -> (字节数, sha256)。2026-11-05 实拉后钉住。
EXPECTED = {
    "model.int8.onnx": (
        81828675,
        "3ef6c19369b912f7caf3cef8e545c5ccd1a33d9d7ec792a46668dc41c4b229ec",
    ),
    "tokens.txt": (
        75352,
        "4b2d964e18b9cf139b473003b6698fb2ed9a2a5ec55b93daa677b28f578897aa",
    ),
}

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(ROOT, "android", "app", "src", "main", "assets", "asr")


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(name, dest):
    url = f"{MIRROR}/{REPO}/resolve/main/{name}"
    tmp = dest + ".part"
    print(f"  {name} <- {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "feihe-fetch/1"})
    with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        done = 0
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
            done += len(chunk)
            if total:
                pct = done * 100 // total
                print(f"\r    {done/1048576:6.1f} / {total/1048576:6.1f} MB  {pct:3d}%",
                      end="", flush=True)
        print()
    # 先落 .part 再改名：中断时不会留下一个「看起来完整」的半截文件
    os.replace(tmp, dest)


def verify_one(name):
    path = os.path.join(OUT_DIR, name)
    if not os.path.exists(path):
        return f"缺失: {name}"
    size = os.path.getsize(path)
    digest = sha256_of(path)
    exp_size, exp_hash = EXPECTED.get(name, (None, None))
    line = f"{name}: {size} bytes  sha256={digest}"
    if exp_size is not None and size != exp_size:
        return line + f"\n  ✗ 大小不符，期望 {exp_size}"
    if exp_hash is not None and digest != exp_hash:
        return line + f"\n  ✗ sha256 不符，期望 {exp_hash}"
    return line + ("  ✓" if exp_size is not None else "  （未钉住，把上面这行填进 EXPECTED）")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verify", action="store_true", help="只校验，不下载")
    args = ap.parse_args()

    if not args.verify:
        os.makedirs(OUT_DIR, exist_ok=True)
        for name in FILES:
            dest = os.path.join(OUT_DIR, name)
            exp_size = EXPECTED.get(name, (None, None))[0]
            if os.path.exists(dest) and (exp_size is None or os.path.getsize(dest) == exp_size):
                print(f"  已存在，跳过: {name}")
                continue
            download(name, dest)

    ok = True
    for name in FILES:
        msg = verify_one(name)
        print(msg)
        if "✗" in msg or "缺失" in msg:
            ok = False
    if not ok:
        print("\n模型不完整，删掉 assets/asr/ 重新跑一次。")
        return 1
    print(f"\n完成：{OUT_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
