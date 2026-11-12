package com.feihe.ime.keyboard

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 退格拆字判据。和 Windows 端 `textservice.rs::backspace_splits` 必须一致——
 * 用户换设备不该重新学一遍手感。
 */
class BackspaceSplitTest {

    @Test
    fun `多字词要拆`() {
        assertTrue(BackspaceSplit.shouldSplit(listOf("璇玑", "悬机")))
        assertTrue(BackspaceSplit.shouldSplit(listOf("你好")))
    }

    @Test
    fun `单字不拆`() {
        // `de` → 的：这码本来就是「一堆同码字」，拆成逐字毫无意义
        assertFalse(BackspaceSplit.shouldSplit(listOf("的", "得", "地")))
    }

    @Test
    fun `没有候选不拆`() {
        assertFalse(BackspaceSplit.shouldSplit(emptyList()))
    }

    @Test
    fun `退化显示成原始编码时不拆`() {
        // 没命中时候选条显示的是码本身，那不是词
        assertFalse(BackspaceSplit.shouldSplit(listOf("nihao")))
        assertFalse(BackspaceSplit.shouldSplit(listOf("xrji", "璇玑")))
    }

    @Test
    fun `三字词四字词都要拆`() {
        assertTrue(BackspaceSplit.shouldSplit(listOf("李明远")))
        assertTrue(BackspaceSplit.shouldSplit(listOf("中华人民共和国")))
    }

    @Test
    fun `含数字的词不拆 避免把验证码之类拆开`() {
        // 单字长度 <2 直接 false；「3D」这种含 ASCII 数字也不是中文词
        assertFalse(BackspaceSplit.shouldSplit(listOf("3D")))
    }

    @Test
    fun `isWord 边界`() {
        assertFalse(BackspaceSplit.isWord(""))
        assertFalse(BackspaceSplit.isWord("我"))
        assertTrue(BackspaceSplit.isWord("我们"))
        assertFalse(BackspaceSplit.isWord("abc"))
        assertFalse(BackspaceSplit.isWord("中文1"))
        assertTrue(BackspaceSplit.isWord("中文"))
    }
}
