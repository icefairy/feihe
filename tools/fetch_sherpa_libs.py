#!/usr/bin/env python3
"""补齐 Android 语音识别的原生库 + VAD 模型（约 31 MB，不进版本库）。

## 为什么不放进 git

和 `tools/fetch_asr_model.py`（79 MB 的 ASR 模型）同一个理由：这些是**构建输入**，
不是源码。放进去每次 clone 都要背一遍，历史里还删不掉。用脚本拉/拷、用 SHA256 钉住。

## 文件来自哪里

`android/app/src/main/java/com/k2fsa/sherpa/onnx/` 里那份 Kotlin API 是 sherpa-onnx
官方 Android 调用层；它下面的 JNI 符号名是**硬编码**的 `com/k2fsa/sherpa/onnx/...`，
所以 `.so` 必须和那份 Kotlin 同一个版本，否则运行期 `UnsatisfiedLinkError`。

本机 `/d/codes/xingyu`（星语）里有**已经在真机上验证过的一套**，版本与这份 Kotlin
API 完全一致，所以默认直接从那儿拷，零网络、零版本漂移。这也是 `docs/Android-方案.md`
§4 说的「能用星语的：集成样板」。

从国内镜像拿也可以：把镜像里 sherpa-onnx 的 arm64 `.so` 和 `silero_vad.onnx` 放到
一个目录，用 `--from <目录>` 指过来即可。脚本只认文件名 + SHA256，不关心来源。
注意镜像里的第三方 AAR（如 `com.bihe0832.android:lib-sherpa-onnx`）可能是旧版 API，
`OfflineModelConfig` 字段对不上会在运行期炸，**务必用校验值确认版本一致**。

## 用法

    python tools/fetch_sherpa_libs.py                 # 从默认样板目录拷
    python tools/fetch_sherpa_libs.py --from DIR     # 从指定目录（含镜像解包结果）拷
    python tools/fetch_sherpa_libs.py --verify       # 只校验，不拷贝

校验不过一律非零退出，`assembleDebug` 之前先跑一次能挡住「换了文件却没发现」。
"""

import argparse
import hashlib
import os
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 本机验证过的样板。Windows / WSL 两种路径都试，找不到就要求 --from。
DEFAULT_SOURCES = [
    os.path.join(os.path.dirname(ROOT), "xingyu", "m1", "android-app", "app", "src", "main"),
    "/data/codes/xingyu/m1/android-app/app/src/main",
]

JNI_DST = os.path.join(ROOT, "android", "app", "src", "main", "jniLibs", "arm64-v8a")
ASSET_DST = os.path.join(ROOT, "android", "app", "src", "main", "assets")

# 文件名 -> (来源子目录, 字节数, sha256)。2026-11-05 从已验证样板实取后钉住。
EXPECTED = {
    "libonnxruntime.so": (
        "jniLibs/arm64-v8a", 21684880,
        "dc5e4c172b1be9e530c6a62ad8f1be3e0a911cabdee6195abf28dab72477e194",
    ),
    "libsherpa-onnx-c-api.so": (
        "jniLibs/arm64-v8a", 4456784,
        "73dba26ddf63e47e6c6e8b7c663d50c834b267ec0bb1ada1bc8e472e390bc41a",
    ),
    "libsherpa-onnx-cxx-api.so": (
        "jniLibs/arm64-v8a", 440760,
        "986dac2b94bf1da10112c2ce69a3f15439e5f04061d5891cf970f2eeaccb213d",
    ),
    "libsherpa-onnx-jni.so": (
        "jniLibs/arm64-v8a", 4761536,
        "be77572bfd01586b0e4531f3a223f8d1f2e6b257e567efe861de1faf47dfe969",
    ),
    "silero_vad.onnx": (
        "assets", 643854,
        "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
    ),
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def dst_for(name):
    return os.path.join(ASSET_DST if name.endswith(".onnx") else JNI_DST, name)


def verify():
    missing, bad = [], []
    for name, (_, size, want) in EXPECTED.items():
        path = dst_for(name)
        if not os.path.isfile(path):
            missing.append(name)
            continue
        got = sha256(path)
        actual = os.path.getsize(path)
        if got != want or actual != size:
            bad.append(f"{name}: sha={got[:12]}… size={actual}")
    for name in missing:
        print(f"缺失 {name}")
    for line in bad:
        print(f"不匹配 {line}")
    if not missing and not bad:
        print(f"OK：{len(EXPECTED)} 个文件齐全且校验通过")
        return 0
    return 1


def find_source(base):
    """在来源目录里按文件名找，兼容「平铺」和「jniLibs/arm64-v8a 分目录」两种解包形状。"""
    found = {}
    for name in EXPECTED:
        for root, _dirs, files in os.walk(base):
            if name in files:
                found[name] = os.path.join(root, name)
                break
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", help="来源目录（镜像解包结果）")
    ap.add_argument("--verify", action="store_true", help="只校验，不拷贝")
    args = ap.parse_args()

    if args.verify:
        return verify()

    if args.src:
        bases = [args.src]
    else:
        bases = [b for b in DEFAULT_SOURCES if os.path.isdir(b)]
        if not bases:
            print("找不到已验证的样板目录，请用 --from 指定来源（或先跑 fetch_asr_model.py）。")
            return 1

    src = None
    for base in bases:
        cand = find_source(base)
        if len(cand) == len(EXPECTED):
            src = cand
            break
    if src is None:
        print(f"来源里没凑齐 {len(EXPECTED)} 个文件：{bases}")
        return 1

    # 先全量校验来源，再动手拷：避免拷到一半发现某个文件不对，留下半套库。
    for name, (_, size, want) in EXPECTED.items():
        path = src[name]
        if os.path.getsize(path) != size or sha256(path) != want:
            print(f"来源文件校验不通过（版本可能不一致）：{name}")
            return 1

    os.makedirs(JNI_DST, exist_ok=True)
    os.makedirs(ASSET_DST, exist_ok=True)
    for name in EXPECTED:
        shutil.copy2(src[name], dst_for(name))
        print(f"拷贝 {name}")

    return verify()


if __name__ == "__main__":
    sys.exit(main())
