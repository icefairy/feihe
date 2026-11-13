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

    /**
     * 长按气泡里的完整列表必须**包含**键面上那两个，否则用户长按发现
     * 「键面写着 X，点开却没有 X」，会觉得这功能是错的。
     */
    @Test
    fun `完整列表包含键面提示的字`() {
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
    }

    /** 未知方案不能崩，返回空串即可。 */
    @Test
    fun `未知方案返回空`() {
        assertEquals("", AssistKeyHints.hintFor('s', "shuangpin"))
        assertEquals("", AssistKeyHints.fullHint('s', ""))
    }
}
