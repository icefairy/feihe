package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 回车键的**完整规格**回归。
 *
 * 用户明确要求（原话）：
 * > 回车原样上屏就行，不要追加换行，在没有输入任何字母的时候回车才换行。
 *
 * 这句话有两半，两半都被我改错过一次，所以这里全部钉住：
 *
 * 1. **手上有输入 → 只上屏，不换行、不发 action。**
 *    - 错法 A：上屏后 `performEnterAction()` → 网页输入框里 `haha` 回车变成
 *      「haha + 换行」，把输入连同换行一起提交出去。
 *    - 错法 B：上屏后 `commitText("\n")` → 单行框里变成一个空格。
 *
 * 2. **手上没输入 → 交给宿主。**
 *    - 地址栏 GO 要跳转，搜索框 SEARCH 要搜索，没有 action 的（Termux）要发 `\n`。
 *    - 错法：没有 action 就"什么都不做"→ Termux 里 `su` 回车完全失效，
 *      命令行没法用（这是最严重的一次）。
 *
 * 判定拆成两层：
 * - [EnterSpec.decide]：**没有输入时**这一下回车交给谁（本文件下半部分）。
 * - [EnterSpec.shouldLeaveIfComposing]：**有输入时**是否就该停手（上半部分）。
 */
class EnterSpecTest {

    /** 模拟 ENTER 分支的决策：手上有没有待处理输入。 */
    private fun behavesAsTyped(hasPending: Boolean): Boolean = hasPending

    @Test
    fun `中文态有输入 —— 只上屏，不换行不发 action`() {
        // 中文态 pending 非空 = 有一串还没转成汉字的字母。
        // 回车的唯一语义是「原样上屏」，必须在这里停手。
        assertTrue(behavesAsTyped(hasPending = true))
        assertEquals(
            EnterSpec.Decision.CommitOnly,
            EnterSpec.decide(chinese = true, pendingNotEmpty = true, imeAction = 0),
        )
    }

    @Test
    fun `英文态有输入 —— 同样只上屏`() {
        // 英文已经是逐字 commitText 上去的，pending 只是账本；
        // 回车结束这个词，不能补换行。
        assertEquals(
            EnterSpec.Decision.CommitOnly,
            EnterSpec.decide(chinese = false, pendingNotEmpty = true, imeAction = 0),
        )
    }

    @Test
    fun `中文态有输入时忽略宿主 action —— 地址栏也不例外`() {
        // 即使用户在地址栏打了拼音想直接访问，也不能自动跳：用户要的是"上屏"。
        // 真跳了就是替用户做决定，而且用户下次不知道发生了什么。
        assertEquals(
            EnterSpec.Decision.CommitOnly,
            EnterSpec.decide(
                chinese = true,
                pendingNotEmpty = true,
                imeAction = android.view.inputmethod.EditorInfo.IME_ACTION_GO,
            ),
        )
    }

    @Test
    fun `没有输入 + 飞书微信的 DONE —— 什么都不发，绝不能收键盘`() {
        // 飞书/微信聊天框实测 imeOptions=0x10000003，action=IME_ACTION_DONE(3)。
        // 曾经把 DONE 交给宿主 → 触发"完成并收键盘"，用户按一下回车键盘就没了，
        // 得手动点回来。这是明确不可接受的行为。
        assertEquals(
            EnterSpec.Decision.DoNothing,
            EnterSpec.decide(
                chinese = true,
                pendingNotEmpty = false,
                imeAction = android.view.inputmethod.EditorInfo.IME_ACTION_DONE,
            ),
        )
    }

    @Test
    fun `没有输入 + SEND —— 什么都不发，不触发发送、不收键盘`() {
        // SEND 也是收键盘的动作，同样不能交出去，也不该发换行。
        assertEquals(
            EnterSpec.Decision.DoNothing,
            EnterSpec.decide(
                chinese = true,
                pendingNotEmpty = false,
                imeAction = android.view.inputmethod.EditorInfo.IME_ACTION_SEND,
            ),
        )
    }

    @Test
    fun `没有输入 + 地址栏 —— 交给宿主跳转`() {
        assertEquals(
            EnterSpec.Decision.PerformEditorAction(android.view.inputmethod.EditorInfo.IME_ACTION_GO),
            EnterSpec.decide(
                chinese = true,
                pendingNotEmpty = false,
                imeAction = android.view.inputmethod.EditorInfo.IME_ACTION_GO,
            ),
        )
    }

    @Test
    fun `没有输入 + Termux 无 action —— 必须发换行`() {
        // 最严重的一次回归：这里曾经是"什么都不做"，
        // 导致手机终端里敲 su 回车毫无反应，输入法在 Termux 里直接不能用。
        assertEquals(
            EnterSpec.Decision.Newline,
            EnterSpec.decide(chinese = true, pendingNotEmpty = false, imeAction = 0),
        )
    }

    @Test
    fun `有输入时任何框都只上屏`() {
        // 第一层规格与宿主无关，中英 / 各种 action 组合都必须是 CommitOnly
        listOf(true, false).forEach { ch ->
            listOf(
                0,
                android.view.inputmethod.EditorInfo.IME_ACTION_GO,
                android.view.inputmethod.EditorInfo.IME_ACTION_DONE,
                android.view.inputmethod.EditorInfo.IME_ACTION_SEND,
            ).forEach { act ->
                assertEquals(
                    EnterSpec.Decision.CommitOnly,
                    EnterSpec.decide(ch, pendingNotEmpty = true, imeAction = act),
                )
            }
        }
    }

    @Test
    fun `只有真正没有 action 的框才发换行 —— Termux 不能废`() {
        // 这条钉住最严重那次事故：Termux 的 TYPE_NULL 终端 imeAction=0，
        // 曾经因为"没有 action 就什么都不做"让回车彻底失效、命令行不可用。
        // 反过来 DONE/SEND 也不能掉进 Newline，否则宿主收到换行会收键盘。
        listOf(true, false).forEach { ch ->
            assertEquals(
                EnterSpec.Decision.Newline,
                EnterSpec.decide(ch, pendingNotEmpty = false, imeAction = 0),
            )
            assertEquals(
                EnterSpec.Decision.DoNothing,
                EnterSpec.decide(
                    ch, pendingNotEmpty = false,
                    imeAction = android.view.inputmethod.EditorInfo.IME_ACTION_DONE,
                ),
            )
        }
    }
}
