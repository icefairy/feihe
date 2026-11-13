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
         * 用于声明了 DONE / SEND 这类「完成」动作的输入框：宿主会把回车
         * 理解成「收工」，连带收起键盘。我们没有需要提交的东西，就不要触发它。
         */
        data object DoNothing : Decision
    }

    /**
     * @param chinese 当前是否中文模式（英文态字母已经逐字上屏，pending 只是账本）
     * @param pendingNotEmpty 手上是否还有没处理完的输入（中文的字母串 / 英文账本）
     * @param imeAction `EditorInfo.imeOptions and EditorInfo.IME_MASK_ACTION`
     * @param multiline 输入框是否声明了 `TYPE_TEXT_FLAG_MULTI_LINE`
     */
    fun decide(
        chinese: Boolean,
        pendingNotEmpty: Boolean,
        imeAction: Int,
        multiline: Boolean = false,
    ): Decision {
        // 第一层：有输入就只上屏。中英两态同一条规格，与宿主是什么框无关。
        if (pendingNotEmpty) return Decision.CommitOnly

        // 第二层：没输入，把这下回车交给宿主。
        //
        // **四层判断，顺序不能换：**
        //   1. 有输入                    → 只上屏（已在上面返回）
        //   2. GO / SEARCH               → 交给宿主（跳转/搜索，都不收键盘）
        //   3. 多行框 + DONE/SEND        → 发换行（飞书/微信聊天，用户要的就是换行）
        //   4. 单行框 + DONE/SEND        → 什么都不发（宿主会"完成并收键盘"）
        //   5. 无 action（Termux 终端）  → 发换行
        //
        // ## 第 3、4 条为什么必须分开
        //
        // 飞书/微信聊天框的 imeOptions 实测是 `0x10000003`：
        // 低字节 DONE(3)，高位 `0x100000` 是 `TYPE_TEXT_FLAG_MULTI_LINE`——**多行框**。
        //
        // 曾经第 3 条也被并进"什么都不发"（为了修"回车收键盘"），
        // 结果飞书里**完全打不出换行**——用一个更常用功能的缺失换了小问题。
        //
        // 区别在发什么：我们发的是**换行字符**，不是 `performEditorAction(DONE)`。
        // 多行框收到换行字符只会插入换行，不会走宿主的"完成"分支，所以不收键盘。
        // 单行框才是任何字符都会触发清洗/提交，那种框里才该什么都不发。
        if (multiline &&
            imeAction != EditorInfo.IME_ACTION_GO &&
            imeAction != EditorInfo.IME_ACTION_SEARCH
        ) {
            return Decision.Newline
        }

        return when (imeAction) {
            // 跳转/搜索：交出去，这两个动作不会收键盘
            EditorInfo.IME_ACTION_GO,
            EditorInfo.IME_ACTION_SEARCH -> Decision.PerformEditorAction(imeAction)

            // 单行框声明 DONE / SEND → 什么都不发。
            // 单行框的 DONE 语义就是"提交并收工"，任何字符都会让它收起键盘。
            // 多行框已在上面走 Newline，不会到这儿。
            EditorInfo.IME_ACTION_DONE,
            EditorInfo.IME_ACTION_SEND,
            EditorInfo.IME_ACTION_NEXT,
            EditorInfo.IME_ACTION_PREVIOUS -> Decision.DoNothing

            // 真·没有声明动作（Termux 的 TYPE_NULL 终端就是这种）：发换行。
            // 这是最严重那次事故的分支，绝不能再改成"什么都不做"之外的东西。
            else -> Decision.Newline
        }
    }
}
