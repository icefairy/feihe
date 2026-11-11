package com.feihe.ime.keyboard

/**
 * 上档状态机 + 数字选词映射。
 *
 * ## 为什么单独抽成纯 Kotlin
 *
 * 这两块逻辑都不碰 Android API，但**特别容易在边界上写错**：上档「一次性 / 锁定」
 * 的区别只在「打完这个字母之后要不要落下」，数字选词则是「第几个候选」的 off-by-one。
 * 放进 `FeiheIme`（InputMethodService 子类）就没法跑普通 JVM 单测了，只能上真机
 * 靠手点——而这两处错了在真机上的表现都是「偶尔不对」，最难复现。
 *
 * 和 `KeyboardLayout` 一样：真值放纯数据里，界面只负责画。
 */

/** 上档的三种状态。`ONCE` 是老行为（按一下、下一个字母大写、打完落下）。 */
enum class ShiftState {
    /** 不激活。 */
    OFF,

    /** 一次性：下一个字母大写，之后自动落回 [OFF]。 */
    ONCE,

    /** 锁定：连续大写，直到用户再点一次 ⇧ 或切中英/换页。 */
    LOCKED,
}

object ShiftMachine {
    /**
     * 双击的判定窗口（毫秒）。
     *
     * 300ms 是 Android 自己 ViewConfiguration.getDoubleTapTimeout() 的同量级取值。
     * 松到 500ms 会把「快速连按两下想打两个大写」误判成锁定；紧到 150ms 则正常人
     * 按不出来——尤其手机上拇指按 ⇧ 这种边角键，两次间隔本来就偏大。
     */
    const val DOUBLE_TAP_MS = 300L

    /**
     * 单击 ⇧。
     *
     * - [ShiftState.OFF] → [ShiftState.ONCE]：进入一次性大写。
     * - [ShiftState.ONCE] / [ShiftState.LOCKED] → [ShiftState.OFF]：再点一下就是取消，
     *   这比「ONCE 点第二下变 LOCKED」直观得多，也避免双击手势的时序歧义。
     *
     * 注意这个函数**不处理双击**——双击要靠调用方比对两次点击的时间戳，见
     * [onTapWithTime]。纯单击语义保持不变，因为单测和其它调用点都依赖它。
     */
    fun onTap(cur: ShiftState): ShiftState = when (cur) {
        ShiftState.OFF -> ShiftState.ONCE
        ShiftState.ONCE, ShiftState.LOCKED -> ShiftState.OFF
    }

    /**
     * 带时间的单击 ⇧，**支持双击锁定**。
     *
     * ## 为什么现在改成双击而不是长按
     *
     * 用户的直接要求是「按两下锁定、再按两下解锁」——这是 iOS / Gboard 的手势，
     * 也是大多数人已经形成的肌肉记忆。之前选长按的理由是「双击会和连打两个大写
     * 抢时间窗口」，但那个顾虑在实测里站不住：
     *
     * - 真要连打两个大写，用户会**按住 ⇧ 打**（锁定态）而不是敲两下 ⇧ 再敲字母；
     * - 长按 ⇧ 在手机上极难按准（它在最角上，长按还容易触发宿主的文本选择）。
     *
     * 所以这一版：**单击 = 一次性，双击 = 锁定，锁定态双击 = 解锁**。长按不再是
     * 锁定入口（[onLongPress] 保留给其它地方用，但键盘上不再绑它）。
     *
     * @param lastTapAt 上一次点击该键的时间戳；首次点击传 0
     * @param now 本次点击时间戳
     * @return 新状态 + 是否被判定为双击（调用方据此决定要不要吃掉这次单击）
     */
    fun onTapWithTime(
        cur: ShiftState,
        lastTapAt: Long,
        now: Long,
    ): Pair<ShiftState, Boolean> {
        val isDouble = lastTapAt != 0L && now - lastTapAt <= DOUBLE_TAP_MS
        if (!isDouble) return onTap(cur) to false
        return when (cur) {
            // 双击时已经把 ONCE 吃成 LOCKED，所以 ONCE 也要走这条路：
            // 用户连点两下，第一次变成 ONCE，第二次就该锁住，而不是又落回 OFF。
            ShiftState.OFF, ShiftState.ONCE -> ShiftState.LOCKED to true
            ShiftState.LOCKED -> ShiftState.OFF to true
        }
    }

    /**
     * 长按 ⇧ = 锁定/解锁。
     *
     * 键盘上**已经不再绑定**（改双击了，见 [onTapWithTime]）。保留这个函数是因为
     * 它有单测、语义也仍然正确，将来要加「长按 ⇧ 弹出上档字符表」之类的功能时
     * 可以直接复用。
     */
    fun onLongPress(cur: ShiftState): ShiftState =
        if (cur == ShiftState.LOCKED) ShiftState.OFF else ShiftState.LOCKED

    /**
     * 打完一个字母之后。只有 [ShiftState.ONCE] 会落下；[ShiftState.LOCKED] 保持。
     *
     * 切中英 / 换页 / 上屏标点这些"上下文断开"的动作不调它，由调用方直接置 [ShiftState.OFF]
     * ——那些地方本来就该把上档一起清掉。
     */
    fun afterChar(cur: ShiftState): ShiftState =
        if (cur == ShiftState.ONCE) ShiftState.OFF else cur

    /** 这个字母要不要大写。锁定态下每个字母都大写。 */
    fun upper(cur: ShiftState): Boolean = cur != ShiftState.OFF
}

/**
 * 光标 / 编辑控制页上的每个动作。
 *
 * ## 为什么用枚举而不是一组 lambda
 *
 * 这个枚举是 `Key.code` 里存的东西，也是 [com.feihe.ime.FeiheIme] 里那个 `when`
 * 的判据——**加了新动作编译器会逼你处理**。用字符串或者 lambda 表，漏一个分支
 * 的表现是「按了没反应」，最难查。
 */
enum class NavAction {
    LEFT,
    RIGHT,
    UP,
    DOWN,
    SELECT_ALL,
    COPY,
    PASTE,
    CUT,

    /** 移到行首 / 行尾：手机上按 ← 一个个挪太慢，长文本编辑刚需。 */
    LINE_START,
    LINE_END,
    ;

    companion object {
        /** 远端存的是枚举名；认不出来就返回 null，调用方忽略这次点击。 */
        fun of(code: String): NavAction? = entries.firstOrNull { it.name == code }
    }
}

/**
 * 光标移动的**纯计算**：给定文本和当前偏移，算出新偏移。碰不到 Android API，能跑 JVM 单测。
 *
 * ## 为什么要按「列」而不是「字符数」上下移
 *
 * 上一行的第 5 个字符和下一行的第 5 个字符在同一列，用户按 ↑ 时期待光标**垂直往上**
 * ——这是所有编辑器的行为。如果按「上一行末尾」跳，光标会跑到行尾，看着像乱跳。
 *
 * ## 边界
 *
 * - 没有上一行/下一行 → 返回 null（调用方什么都不做，比跳到文本开头好）
 * - 目标行比当前列短 → 落在**该行行尾**（编辑器通行做法）
 */
object NavCursor {

    fun move(text: CharSequence, pos: Int, action: NavAction): Int? {
        val safe = pos.coerceIn(0, text.length)
        return when (action) {
            NavAction.LEFT -> (safe - 1).coerceAtLeast(0)
            NavAction.RIGHT -> (safe + 1).coerceAtMost(text.length)
            NavAction.UP -> moveVertical(text, safe, -1)
            NavAction.DOWN -> moveVertical(text, safe, +1)
            // 其余动作不是移动，交回调用方处理
            else -> null
        }
    }

    /** 行首（该行第一个字符的偏移）。第一行就是 0。 */
    fun lineStart(text: CharSequence, pos: Int): Int {
        val safe = pos.coerceIn(0, text.length)
        if (safe == 0) return 0
        val nl = text.lastIndexOf('\n', safe - 1)
        return if (nl < 0) 0 else nl + 1
    }

    /** 行尾（换行符的偏移，或文本末尾）。 */
    fun lineEnd(text: CharSequence, pos: Int): Int {
        val safe = pos.coerceIn(0, text.length)
        val nl = text.indexOf('\n', safe)
        return if (nl < 0) text.length else nl
    }

    /**
     * 上下移一行，保持列号。
     *
     * `delta` = -1 上一行 / +1 下一行。目标行不存在返回 null。
     */
    private fun moveVertical(text: CharSequence, pos: Int, delta: Int): Int? {
        val starts = lineStarts(text)
        val line = lineOf(text, pos)
        val target = line + delta
        if (target < 0 || target >= starts.size) return null
        val col = pos - starts[line]
        val targetStart = starts[target]
        // 目标行的行尾 = 下一行起点 - 1（换行符的位置）。最后一行的行尾是 text.length。
        val targetEnd = lineEnd(text, targetStart)
        // 目标行比当前列短 → 停在该行行尾（不跨到下一行）
        return (targetStart + col).coerceAtMost(targetEnd)
    }

    /** 光标在第几行（0 起）。 */
    fun lineOf(text: CharSequence, pos: Int): Int {
        var line = 0
        for (i in 0 until pos.coerceIn(0, text.length)) {
            if (text[i] == '\n') line++
        }
        return line
    }

    /** 每一行的起始偏移。空文本返回 [0]。 */
    fun lineStarts(text: CharSequence): List<Int> {
        val out = mutableListOf(0)
        for (i in text.indices) if (text[i] == '\n') out.add(i + 1)
        return out
    }
}

object NumberSelect {
    /** 候选最多给 9 个，和桌面端 `1`–`9` 选词一致。 */
    const val MAX = 9

    /**
     * 当前该显示哪些数字键：`1..min(候选数, 9)`。
     *
     * 没候选时返回空表 → 整行隐藏（**不常驻**），这正是 A2 里「做成可隐藏」的意思：
     * 它只在组合态占一行高度，键盘收起来后字母键一dp不少。
     */
    fun digits(candidateCount: Int): IntRange =
        if (candidateCount <= 0) IntRange.EMPTY else 1..minOf(candidateCount, MAX)

    /**
     * 数字键 → 候选下标（0-based）。越界返回 null，调用方直接忽略。
     *
     * 单独抽出来是因为这里必然有个 `-1`，写进 `onKey` 里就是那种"看起来对、
     * 真机上点 1 选了第二个"的 bug。
     */
    fun indexOf(digit: Int, candidateCount: Int): Int? {
        if (digit !in 1..MAX || digit > candidateCount) return null
        return digit - 1
    }
}
