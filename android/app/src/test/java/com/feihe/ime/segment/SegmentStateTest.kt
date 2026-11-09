package com.feihe.ime.segment

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 逐段选择的规格锁。
 *
 * 重点不是「能不能选」，是**改了某一段之后其它段不能被带跑**，以及
 * 「默认什么都不点」必须和旧版自动整句上屏一字不差。
 */
class SegmentStateTest {

    private val two = SegmentState(
        segments = listOf(
            listOf("张三", "张四", "章三"),
            listOf("李", "里", "理"),
        ),
    )

    @Test
    fun `默认整句就是每段第一名`() {
        assertEquals("张三李", two.sentence())
    }

    @Test
    fun `至少两段且每段非空才算分段态`() {
        assertTrue(two.active)
        assertFalse("只有一段", SegmentState(listOf(listOf("你好"))).active)
        assertFalse("没有段", SegmentState(emptyList()).active)
        assertFalse("有一段空", SegmentState(listOf(listOf("张"), emptyList())).active)
    }

    @Test
    fun `展开一段后能看到该段候选`() {
        val s = two.open(1)
        assertEquals(1, s.focus)
        assertEquals(listOf("李", "里", "理"), s.focusedWords())
    }

    @Test
    fun `选完自动回到 chip 视图`() {
        val s = two.open(1).choose(2)
        assertEquals(-1, s.focus)
        assertEquals("张三理", s.sentence())
    }

    @Test
    fun `改一段不影响其它段`() {
        val s = two.open(0).choose(1)
        assertEquals(listOf(1, 0), s.selected)
        assertEquals("张四李", s.sentence())
    }

    @Test
    fun `两段分别改`() {
        val s = two.open(0).choose(2).open(1).choose(1)
        assertEquals("章三里", s.sentence())
    }

    @Test
    fun `越界操作原样返回`() {
        assertEquals(two, two.open(-1))
        assertEquals(two, two.open(9))
        assertEquals("没展开时不该选", two, two.choose(0))
        assertEquals(two.open(0), two.open(0).choose(99))
    }

    @Test
    fun `back 收起但不改选择`() {
        val s = two.open(1).choose(2).open(0).back()
        assertEquals(-1, s.focus)
        assertEquals("张三理", s.sentence())
    }

    @Test
    fun `wordAt 越界返回空串`() {
        assertEquals("张三", two.wordAt(0))
        assertEquals("", two.wordAt(9))
    }

    /**
     * 形码态按**词**选：界面上的下标来自筛过的子集，不能拿去索引原列表。
     * 这条错了的表现是「点了甲，上去的是乙」——而且是间歇性的（取决于筛出几个）。
     */
    @Test
    fun `chooseWord 按词定位下标`() {
        val s = two.open(0).chooseWord("章三")
        assertEquals(2, s.selected[0])
        assertEquals("章三李", s.sentence())
        assertEquals("选完回到 chip 视图", -1, s.focus)
    }

    /** 形码态跨段保持：选完自动展开下一段，最后一段则收起。 */
    @Test
    fun `chooseWord keepOpen 展开下一段`() {
        val s = two.open(0).chooseWord("张三", keepOpen = true)
        assertEquals(1, s.focus)
        val last = s.chooseWord("李", keepOpen = true)
        assertEquals(-1, last.focus)
        assertEquals("张三李", last.sentence())
    }

    /** 词不在这段里（比如筛空后点到了残影）→ 原样返回，什么都不改。 */
    @Test
    fun `chooseWord 不认识的词不动状态`() {
        val s = two.open(0)
        assertEquals(s, s.chooseWord("不存在"))
        assertEquals(two, two.chooseWord("张三"))
    }
}
