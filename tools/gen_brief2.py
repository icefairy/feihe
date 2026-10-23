#!/usr/bin/env python3
"""生成二简词表（`src/brief2.txt` 的数据行）

二简词的码 = **首字首码 + 次字首码**，也就是小鹤双拼两键码的第一个键：

    zh→v  ch→i  sh→u   单字母声母取自身   零声母（a/e/o 开头）取韵母首字母

所以「难道」= n(nan) + d(dao) = `nd`，「所以」= s(suo) + y(yi) = `sy`。

码不手抄：370 个两字母码手抄必错，而错了不报错——只是那个词永远不出来。
这里用 `src/dict.txt` 里每个词的拼音现算，缺词的在 MANUAL 里补拼音。

输出格式（UTF-8，按码升序）：

    码 词            ← 首选
    码 词 次          ← 次选（码本来归某个更常用的单字）

文件开头的 `#` 注释块（表头说明）原样保留，只替换数据行。

用法：
    python tools/gen_brief2.py                  # 重写 src/brief2.txt
    python tools/gen_brief2.py --check          # 只比对，不写盘（CI 用）
"""

import io
import sys

DICT = "src/dict.txt"
OUT = "src/brief2.txt"

# 小鹤入门 2.1「二简词」首选 273
PRIMARY = """
难道 一定 因为 拼音 简单 所以 我们 从来 只能 习惯 反正 五笔 出现 可能 需要 解决 非常 问题 成为 结果 双拼 明显 容易 自己 现在 发现 简直 方便 相当 喜欢
本来 准备 首先 统一 了解 一下 目前 知道 根本 浪费 一直 认为 努力 才能 掌握 这样 东西 怎么 这么 随便
而且 唯一 要求 联系 地方 应该 正确 反应 怎样 完成 云南 美丽 台湾 传统 欧洲 漂亮 日本 安静 湖南 热闹
西安 骄傲 昆明 特别 西欧 奇怪 人类 希望 自然 昂贵 昨天 烦恼 今天 折腾 明天 精彩 后天 放松 天天 安逸
两天 三天 下周 安排 每周 天气 人们 热爱 无线 现金 为了 安全 算了 成本 走了 脑子 到了 解放 分类 金额
满意 事情 放弃 沟通 思考 错误 分配 公平 承诺 好处 产品 功能 完全 不同 在线 平台 参考 微软 额外 必要
儿子 原来 那些 同学 似乎 开头 认同 今晚 偶像 代表 貌似 聪明 而不 郁闷 头发 缺乏 爱惜 脾气 反而 儿科
重码 日期 为此 千万 保证 讨论 期间 不再 反对 恩赐 作品 思想 过程 怎能 害怕 节奏 切换 身体 按错 从不
反馈 亲自 按照 上面 培训 认真 计算 配置 确定 位置 过去 比赛 必须 那样 无奈 不然 回去 只怕 暗自 呕吐
下次 采取 安保 按时 配送 普洱 暂且 随你 索取 碎片 繁体 文字 儿童 偶尔 认可 其中 一片 认得 安慰 才行
况且 人品 偶然 开车 而非 顺便 重启 何必 全额 否认 默认 二者 恩爱 从头 神似 安分 所谓 暧昧 几乎 缘分
而是 而已 按理 后面 如下 咋不 代替 拗口 奇偶 凹凸 噩梦 恶劣 恶心 恶毒 恶搞 恶评 险恶 不饿 挨饿 饿死
黯然 猥琐 恐怖 恐怕 唯恐 骗人 讽刺 按到 群殴 暗算 并且 欧阳 干嘛 存盘 网盘 耳机 其次 窗口 相反 方面
最快 收到 什么
"""

# 官方「二简词」次选 97
SECONDARY = """
刚刚 慢慢 渐渐 试试 尝尝 恰恰 背景 伤害 程度 附件 支持 选择 学习 情况 每天 电脑 公司 速度 完美 悠闲
修改 修正 回复 感觉 面前 前面 下面 假如 更新 推荐 推导 达到 链接 追求 困难 极其 极致 极为 束缚 搜索
这些 总算 淘汰 投资 虽然 真正 绝对 任务 发送 规则 否则 尽管 仍然 分别 直到 味道 歧视 从前 征服 世纪
这次 微信 推广 主意 意志 小时 只是 只会 未知 座位 参加 反悔 打算 随着 等着 跟着 分钟 总共 一边 两边
与其 食物 即使 正在 确认 按键 举例 周围 最终 基本 直接 即可 分享 当然 两位 其他 几天
"""

# 词库里没有、或拼音需要手工指定的词
MANUAL = {
    "双拼": "shuang pin", "本来": "ben lai", "统一": "tong yi", "日本": "ri ben",
    "到了": "dao le", "爱惜": "ai xi", "重码": "chong ma", "恩赐": "en ci",
    "按错": "an cuo", "安保": "an bao", "随你": "sui ni", "恩爱": "en ai",
    "神似": "shen si", "安分": "an fen", "按理": "an li", "咋不": "za bu",
    "拗口": "ao kou", "奇偶": "ji ou", "凹凸": "ao tu", "恶毒": "e du",
    "恶评": "e ping", "险恶": "xian e", "不饿": "bu e", "挨饿": "ai e",
    "唯恐": "wei kong", "按到": "an dao", "群殴": "qun ou", "暗算": "an suan",
    "存盘": "cun pan", "网盘": "wang pan", "渐渐": "jian jian",
    "推导": "tui dao", "反悔": "fan hui",
}

# 官方表里「上面」和「什么」撞同一个码（都是 um）。「什么」更常用，让它当首选。
TIER_OVERRIDE = {("um", "上面"): 1}


def head(syl):
    """音节 → 双拼首键"""
    syl = syl.lower()
    if syl[:2] in ("zh", "ch", "sh"):
        return {"zh": "v", "ch": "i", "sh": "u"}[syl[:2]]
    return syl[0]


def load_dict():
    """返回 (两字词 → 拼音, 单字码集合)；单字码用来报告撞码"""
    words = {}
    finals = {
        "a": "a", "o": "o", "e": "e", "i": "i", "u": "u", "v": "v",
        "ai": "d", "ei": "w", "ao": "c", "ou": "z", "an": "j", "en": "f",
        "ang": "h", "eng": "g", "ong": "s", "ia": "x", "ie": "p", "iao": "n",
        "iu": "q", "ian": "m", "in": "b", "iang": "l", "ing": "k", "iong": "s",
        "ua": "x", "uo": "o", "uai": "k", "ui": "v", "uan": "r", "un": "y",
        "uang": "l", "ue": "t", "ve": "t", "van": "r", "vn": "y", "er": "r",
    }
    initials = set("bpmfdtnlgkhjqxrzcsyw")

    def full_code(syl):
        s = syl.lower()
        if s[:2] in ("zh", "ch", "sh"):
            ini, fin = {"zh": "v", "ch": "i", "sh": "u"}[s[:2]], s[2:]
        elif s[0] in initials:
            ini, fin = s[0], s[1:]
        else:
            ini, fin = None, s
        k = finals.get(fin)
        if not k:
            return None
        if ini:
            return ini + k
        if s[0] not in "aeo":
            return None
        return s[0] + (fin[1] if len(fin) == 2 else k)

    single = {}
    for line in io.open(DICT, encoding="utf-8"):
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 2:
            continue
        w, p = parts
        syls = p.split()
        if len(w) == 2 and len(syls) == 2 and w not in words:
            words[w] = p
        elif len(w) == 1 and len(syls) == 1:
            c = full_code(syls[0])
            if c and len(c) == 2:
                single.setdefault(c, w)
    return words, single


def build():
    words, single = load_dict()

    def code_of(w):
        p = MANUAL.get(w) or words.get(w)
        if not p:
            return None
        syls = p.split()
        return head(syls[0]) + head(syls[1]) if len(syls) == 2 else None

    out, missing = [], []
    for tier, text in ((0, PRIMARY), (1, SECONDARY)):
        for w in text.split():
            c = code_of(w)
            if not c:
                missing.append(w)
                continue
            out.append((c, w, TIER_OVERRIDE.get((c, w), tier)))
    if missing:
        sys.exit("这些词在 dict.txt 里查不到拼音，补进 MANUAL：%s" % missing)

    out.sort(key=lambda t: (t[0], t[2]))
    seen = {}
    for c, w, t in out:
        if (c, t) in seen:
            sys.exit("撞码：%s %s 和 %s 同为第 %d 档" % (c, w, seen[(c, t)], t))
        seen[(c, t)] = w

    lines = ["%s %s%s" % (c, w, " 次" if t else "") for c, w, t in out]
    hit = [(c, w, t, single[c]) for c, w, t in out if c in single]
    print("共 %d 条，其中 %d 条撞合法音节码（二简词优先，单字退一位）"
          % (len(lines), len(hit)), file=sys.stderr)
    return lines


def main():
    lines = build()
    body = "\n".join(lines) + "\n"
    if "--check" in sys.argv:
        old = io.open(OUT, encoding="utf-8").read()
        cur = [l for l in old.splitlines() if l.strip() and not l.startswith("#")]
        if cur != lines:
            sys.exit("%s 与词表不一致，跑一次 python tools/gen_brief2.py" % OUT)
        print("%s 已是最新" % OUT)
        return

    old = io.open(OUT, encoding="utf-8").read()
    header = [l for l in old.splitlines() if l.startswith("#")]
    io.open(OUT, "w", encoding="utf-8", newline="\n").write(
        "\n".join(header) + "\n" + body)
    print("写入 %s（%d 条）" % (OUT, len(lines)))


main()
