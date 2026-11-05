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
}
