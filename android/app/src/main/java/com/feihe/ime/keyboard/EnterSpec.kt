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
     */
    fun decide(chinese: Boolean, pendingNotEmpty: Boolean, imeAction: Int): Decision {
        // 第一层：有输入就只上屏。中英两态同一条规格，与宿主是什么框无关。
        if (pendingNotEmpty) return Decision.CommitOnly

        // 第二层：没输入，把这下回车交给宿主。
        //
        // **白名单：只有 GO / SEARCH 交出去，DONE/SEND 类什么都不发，其余换行。**
        //
        // 为什么不能"任何非 NONE 的 action 都交给宿主"：飞书/微信的聊天输入框
        // 声明的是 IME_ACTION_DONE(3)（实测 imeOptions=0x10000003）。
        // 把 DONE 交出去 = 触发宿主"完成并收起键盘"，用户看到的就是
        // **按一下回车键盘自己消失了，还得手动点回来**。
        //
        // 只有 GO / SEARCH 这种"提交后还要继续用键盘"的动作才值得交出去：
        // 地址栏 GO 跳转、搜索框 SEARCH 搜索，两者都不收键盘。
        // SEND / DONE / NEXT / PREVIOUS 是"完成类"动作，宿主处理它们时会收键盘
        // 或挪焦点，都会打断连续输入，所以什么都不发。
        return when (imeAction) {
            // 跳转/搜索：交出去，这两个动作不会收键盘
            EditorInfo.IME_ACTION_GO,
            EditorInfo.IME_ACTION_SEARCH -> Decision.PerformEditorAction(imeAction)

            // **DONE / SEND 等"完成类"动作：什么都不发。**
            //
            // 这类框（飞书/微信聊天、Settings 搜索）声明 DONE 的意思是
            // 「回车 = 完成这一轮输入」。宿主对它的处理**包括收起键盘**。
            // 实测飞书/微信的 imeOptions=0x10000003（action=DONE），
            // 只要往这种框里发换行符或 performEditorAction(DONE)，键盘立刻消失，
            // 用户得手动点回来——这就是「回车后键盘自己收起」的根因。
            //
            // 用户要的只是"把手上那串字母上屏"，上屏已经在上一层做完了；
            // 这一层没有输入可上，就不该再触发宿主的完成逻辑。
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
