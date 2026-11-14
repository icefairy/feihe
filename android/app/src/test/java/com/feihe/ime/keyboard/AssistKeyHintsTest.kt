package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 形码键位提示的契约测试。
 *
 * 这些提示直接印在按键上给用户看，**错了不会报错、只会让人照着按却筛不出字**——
 * 那比没有提示更糟。所以每一条都必须有确定性来源可核对。
 *
 * v0.3.85 那版小鹤表是手写推算的，16/26 键带「日」、`月月`/`日日` 自重复，
 * 用户看到「每个按键都有 日」。这一版改成小鹤官网「部件字根」表，下面几条
 * 专门钉死「不许再退化成兜底字 / 复制自然码」。
 */
class AssistKeyHintsTest {

    @Test
    fun `off 方案不产生任何提示`() {
        assertFalse(AssistKeyHints.hasHints("off"))
        assertEquals("", AssistKeyHints.hintFor('s', "off"))
        assertEquals("", AssistKeyHints.fullHint('s', "off"))
    }

    @Test
    fun `两套方案都声明为有提示`() {
        assertTrue(AssistKeyHints.hasHints("ziranma"))
        assertTrue(AssistKeyHints.hasHints("xiaohe"))
    }

    @Test
    fun `自然码键面提示是两个字`() {
        // 键宽只够两个字，三个字开始挤成一团看不清
        for (c in 'a'..'z') {
            val h = AssistKeyHints.hintFor(c, "ziranma")
            assertTrue("键 $c 的提示「$h」超过两个字", h.length <= 2)
        }
    }

    /**
     * 自然码的 a/i/o/u/v 五个键**没有**部首映射——214 个康熙部首里没有读音
     * 以这五个字母开头的。这不是漏了，是数据事实。
     */
    @Test
    fun `自然码五个键没有提示`() {
        for (c in listOf('a', 'i', 'o', 'u', 'v')) {
            assertEquals("键 $c 不该有自然码提示", "", AssistKeyHints.hintFor(c, "ziranma"))
        }
    }

    /** 其余 21 个键都必须有提示，否则用户按到那儿一片空白会以为坏了。 */
    @Test
    fun `自然码其余21个键都有提示`() {
        val expected = "bcdefghjklmnpqrstwxyz"
        for (c in expected) {
            assertTrue("键 $c 缺自然码提示", AssistKeyHints.hintFor(c, "ziranma").isNotEmpty())
        }
        assertEquals(21, expected.length)
    }

    @Test
    fun `小鹤音形26个键全覆盖`() {
        for (c in 'a'..'z') {
            assertTrue("键 $c 缺小鹤提示", AssistKeyHints.hintFor(c, "xiaohe").isNotEmpty())
        }
    }

    /** 键面最多两个字（官方只有一个部件的键就显示一个，不硬凑）。 */
    @Test
    fun `小鹤键面提示不超过两个字`() {
        for (c in 'a'..'z') {
            val h = AssistKeyHints.hintFor(c, "xiaohe")
            assertTrue("键 $c 的提示「$h」超过两个字", h.length <= 2)
            assertTrue("键 $c 提示不该为空", h.isNotEmpty())
        }
    }

    /**
     * 「每个按键都有 日」的回归闸门。
     *
     * v0.3.85 的手写表把「日」当兜底撒在 16/26 个键上。官方部件表里「日」
     * 只属于 `o`（日月目）这一键。给一点冗余：允许「日」最多出现在 2 个键
     * （万一将来官方表把含日部件归到别键），但绝不能十几个键都有。
     */
    @Test
    fun `小鹤键面不许把日撒满键盘`() {
        val withRi = ('a'..'z').filter { '日' in AssistKeyHints.hintFor(it, "xiaohe") }
        assertTrue(
            "「日」出现在 ${withRi.size} 个键上：${withRi.joinToString("")}。" +
                "这是 v0.3.85 手写兜底表的老 bug，官方表里日只在 o 键。",
            withRi.size <= 2,
        )
    }

    /** 不许再出现 `日日` / `月月` 这种两字相同的自重复（旧表 o/r/v 都犯过）。 */
    @Test
    fun `小鹤键面不许两字相同`() {
        for (c in 'a'..'z') {
            val h = AssistKeyHints.hintFor(c, "xiaohe")
            assertFalse(
                "键 $c 提示「$h」两字相同——旧表 o/r/v=日日、e=月月 就是这么错的",
                h.length == 2 && h[0] == h[1],
            )
        }
    }

    /** 键面必须和自然码表明显不同（旧版 g=广工、m=木目 直接抄了自然码）。 */
    @Test
    fun `小鹤键面不是自然码的复制`() {
        var same = 0
        for (c in 'a'..'z') {
            val a = AssistKeyHints.hintFor(c, "xiaohe")
            val b = AssistKeyHints.hintFor(c, "ziranma")
            if (a.isNotEmpty() && a == b) same++
        }
        assertTrue("小鹤与自然码有 $same 个键提示完全相同，疑似复制错表", same <= 2)
    }

    /** 官方部件表的几个锚点，逐字钉死数据来源（来自 flypy.cc/help zg.md）。 */
    @Test
    fun `小鹤键面锚点与官方部件表一致`() {
        val official = mapOf(
            'a' to "鱼", 'c' to "艹廾", 'j' to "钅金", 'o' to "日月",
            'r' to "亻", 's' to "纟厶", 't' to "田", 'v' to "竹豸",
            'w' to "攵文", 'x' to "忄乂", 'y' to "讠羊", 'z' to "辶足",
        )
        for ((k, v) in official) {
            assertEquals("键 $k 与官方部件表不符", v, AssistKeyHints.hintFor(k, "xiaohe"))
        }
    }

    /** 长按气泡的完整表必须比键面长（或相等），且不再等于键面两个字。 */
    @Test
    fun `小鹤完整表至少不短于键面`() {
        for (c in 'a'..'z') {
            val short = AssistKeyHints.hintFor(c, "xiaohe")
            val full = AssistKeyHints.fullHint(c, "xiaohe")
            assertTrue("键 $c 完整表「$full」比键面「$short」还短", full.length >= short.length)
        }
        // s 键官方有 纟厶罒 等多个，气泡该摊开而不是只给键面两个
        assertTrue(AssistKeyHints.fullHint('s', "xiaohe").length >= 3)
    }

    /** 官方 o 键就是「日月目」——「日」在这里出现是数据事实，不是兜底。 */
    @Test
    fun `日的正确归属是 o 键`() {
        assertTrue('日' in AssistKeyHints.hintFor('o', "xiaohe"))
        assertTrue(AssistKeyHints.fullHint('o', "xiaohe").contains("日月目"))
    }

    /**
     * 自然码完整列表里的每个键面字都得在全表里（这套是确定性反查，不该漏）。
     */
    @Test
    fun `自然码完整列表包含键面提示的字`() {
        for (c in 'a'..'z') {
            val short = AssistKeyHints.hintFor(c, "ziranma")
            if (short.isEmpty()) continue
            val full = AssistKeyHints.fullHint(c, "ziranma")
            for (ch in short) {
                assertTrue("键 $c 键面有「$ch」但完整列表「$full」里没有", full.contains(ch))
            }
        }
    }

    /** 自然码完整列表应该比键面长得多（键面 2 个，全表动辄 20+）。 */
    @Test
    fun `自然码完整列表显著更长`() {
        val sFull = AssistKeyHints.fullHint('s', "ziranma")
        assertTrue("s 键的完整部首表应有 20 个以上，实际 ${sFull.length}", sFull.length > 20)
    }

    /** 大小写都要能用（键盘内部传的是小写，但别依赖调用方）。 */
    @Test
    fun `大小写都返回同样结果`() {
        assertEquals(
            AssistKeyHints.hintFor('s', "ziranma"),
            AssistKeyHints.hintFor('S', "ziranma"),
        )
        assertEquals(
            AssistKeyHints.hintFor('o', "xiaohe"),
            AssistKeyHints.hintFor('O', "xiaohe"),
        )
    }

    /** 未知方案不能崩，返回空串即可。 */
    @Test
    fun `未知方案返回空`() {
        assertEquals("", AssistKeyHints.hintFor('s', "shuangpin"))
        assertEquals("", AssistKeyHints.fullHint('s', ""))
    }
}
