package com.feihe.ime.keyboard

import android.view.inputmethod.EditorInfo

/**
 * 回车键的**完整规格**——两层判定合一，供 [com.feihe.ime.FeiheIme] 的 ENTER 分支调用。
 *
 * ## 用户的原始规格（原话）
 *
 * > 回车原样上屏就行，不要追加换行，在没有输入任何字母的时候回车才换行。
 *
 * 拆成两层：
 *
 * 1. **手上有待处理输入 → [Decision.CommitOnly]**：只上屏，绝不换行、不发 action。
 * 2. **手上没有输入 → 看宿主声明的 IME action**：
 *    - 有 GO / SEARCH / SEND… → [Decision.PerformEditorAction]
 *    - 没有（NONE / UNSPECIFIED / TYPE_NULL 的 Termux）→ [Decision.Newline]
 *
 * ## 这段逻辑为什么值得单独成文件
 *
 * 它在两天里被我改坏了三次，每次都是**默认分支选错**，而且症状差异极大：
 *
 * | 错法 | 症状 |
 * |---|---|
 * | 有输入时也发 action | 网页里敲 `haha` 回车 → `haha` + 换行一起提交 |
 * | 有输入时补 `\n` | 单行框里变成一个空格，地址栏不跳转 |
 * | 没输入且无 action → 什么都不做 | **Termux 里回车彻底失效，`su` 敲完没反应** |
 *
 * 三种症状看起来毫无关系（一个是多换行、一个是空格、一个是没反应），
 * 根因却是同一处 if 的边界。抽成纯函数后用单测把所有组合跑一遍，
 * 比在真机上碰运气可靠——真机上你只有一两个场景，而输入框有十几种。
 */
object EnterSpec {

    /** 这一下回车该做什么。 */
    sealed interface Decision {
        /**
         * 手上有待处理输入：把它原样上屏，然后**停手**。
         * 不换行、不发 action。
         */
        data object CommitOnly : Decision

        /** 交给宿主的 IME action（地址栏 GO / 搜索框 SEARCH / 聊天 SEND）。 */
        data class PerformEditorAction(val action: Int) : Decision

        /** 发一个换行。**没有 action 时的默认动作**，Termux 靠它才能用。 */
        data object Newline : Decision

        /**
         * 什么都不做。
         *
         * **任何情况下都不该返回这个值。** 保留它只是为了在测试里断言
         * "没有输入时绝不能落到这个分支"——那个分支曾经让 Termux 完全不可用。
         */
        data object DoNothing : Decision
    }

    /**
     * @param chinese 当前是否中文模式（英文态字母已经逐字上屏，pending 只是账本）
     * @param pendingNotEmpty 手上是否还有没处理完的输入（中文的字母串 / 英文账本）
     * @param imeAction `EditorInfo.imeOptions and EditorInfo.IME_MASK_ACTION`
     */
    fun decide(chinese: Boolean, pendingNotEmpty: Boolean, imeAction: Int): Decision {
        // 第一层：有输入就只上屏。中英两态同一条规格，与宿主是什么框无关。
        if (pendingNotEmpty) return Decision.CommitOnly

        // 第二层：没输入，把这下回车交给宿主。
        // **默认是 Newline，不是 DoNothing**——理由见类注释的第三条。
        return when (imeAction) {
            EditorInfo.IME_ACTION_NONE,
            EditorInfo.IME_ACTION_UNSPECIFIED,
            0 -> Decision.Newline
            else -> Decision.PerformEditorAction(imeAction)
        }
    }
}
