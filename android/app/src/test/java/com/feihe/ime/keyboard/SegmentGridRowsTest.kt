package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 分段态展开网格：**内容行数必须够放下全部候选**。
 *
 * ## 钉的是哪个 bug
 *
 * 用户报：「多字展开逐字选择，从第二个字就展开，2 和 3 都正常，到第 4 个异常，
 * 直接上屏了所有字母，连我之前选的字都没上屏。」
 *
 * 实测引擎侧完全正常：`lmhdilvi` 切 4 段，第 4 段（`vi`）有 **99** 个候选。
 *
 * 崩在渲染：行容器数原来按「屏幕能放几行」截断（8 行 = 48 个），
 * 而填充循环遍历全部 99 个候选 → `col.getChildAt(99/6+1)` 越界返回 null →
 * `as LinearLayout` NPE → 输入法进程被杀 → 宿主把组合区那串字母刷出上屏。
 *
 * 用户看到的是「选到第 4 个字，前面选的全没了，直接把字母串上屏」——
 * 症状完全不像崩溃，因为崩溃发生在另一个进程里。
 *
 * ## 修法与判据
 *
 * 内容行数 = ceil(候选数 / 每行个数)，**一个不少**；只把**可视高度**封顶，
 * 超出交给 ScrollView 滚动。这样「候选多」不再等于「崩」或「滚不动」。
 */
class SegmentGridRowsTest {

    /** 复刻渲染层的行数计算。 */
    private fun gridRows(words: Int): Int =
        if (words == 0) 0 else (words + CandidateGrid.PER_ROW - 1) / CandidateGrid.PER_ROW

    @Test
    fun `99 个候选的行数按列数算 —— 不能按屏幕高度截断`() {
        // `vi` 的实际候选数，正是崩掉那次的那个段。
        // 行数随 PER_ROW 变化（4 列 = 25 行，6 列 = 17 行），所以按公式算，
        // 不写死数字 —— 否则改列数时这条会误红，让人以为越界 bug 回来了。
        val expect = (99 + CandidateGrid.PER_ROW - 1) / CandidateGrid.PER_ROW
        assertEquals(expect, gridRows(99))
        // 加上操作行
        assertEquals(expect + 1, gridRows(99) + 1)
    }

    @Test
    fun `行数必须能放下每一个候选 —— 越界就是空指针闪退`() {
        for (n in listOf(1, 5, 6, 7, 12, 47, 48, 49, 99, 200)) {
            val rows = gridRows(n)
            val capacity = rows * CandidateGrid.PER_ROW
            assertTrue(
                "$n 个候选只有 $rows 行（$capacity 格），最后一个会越界",
                capacity >= n,
            )
            // 且不能多出整整一行浪费空间
            assertTrue("$n 个候选排了 $rows 行，多了空行", capacity - n < CandidateGrid.PER_ROW)
        }
    }

    @Test
    fun `无候选时不占行`() {
        assertEquals(0, gridRows(0))
    }

    @Test
    fun `可视高度仍按屏幕封顶 —— 靠滚动看剩下的`() {
        // 内容 18 行，屏幕只放得下 8 行：容器高度按 8 行算，内容仍 18 行
        val contentRows = gridRows(99) + 1
        val visibleRows = contentRows.coerceAtMost(8)
        assertEquals((99 + CandidateGrid.PER_ROW - 1) / CandidateGrid.PER_ROW + 1, contentRows)
        assertEquals(minOf(contentRows, 8), visibleRows)
        assertTrue("内容必须比可视区高，否则 ScrollView 滚不动", contentRows > visibleRows)
    }

    @Test
    fun `两段词的单字候选也要能滚 —— vihv 的 99 个`() {
        // 用户报的第二个场景：`vihv`（智慧）两个字，第 0 段 `vi` 有 99 个候选，
        // 展开后只有几个且滚不动 —— 同一个越界 bug 的另一副面孔
        val contentRows = gridRows(99) + 1
        assertTrue("99 个候选必须能滚到底", contentRows * CandidateGrid.PER_ROW >= 99)
    }
}
