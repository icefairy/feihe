package com.feihe.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 引导页的两步状态判断。
 *
 * ## 钉的是什么
 *
 * 引导页最容易写错的不是 UI，是**状态判断**。三种真实场景：
 *
 * 1. 新装：两步都没做
 * 2. 做完第一步没做第二步（很常见——切去系统设置勾完就退出了）
 * 3. 两步都做完，**之后又把输入法换回别的**（真实用户会这样，比如临时用系统输入法
 *    打英文）
 *
 * 如果按「第一次进来显示第一步，点完显示第二步」这种流程记忆来做，上面三种里
 * 会显示错两种。所以判据必须每次从系统真实值现算——这个文件钉的就是那个计算。
 *
 * ## 系统里的原始格式
 *
 * `ENABLED_INPUT_METHODS` 形如：
 * ```
 * com.feihe.ime/.FeiheIme:com.android.inputmethod.latin/.LatinIME
 * ```
 * 多项用 `:` 分隔，每项是 `包名/类名`。
 *
 * `DEFAULT_INPUT_METHOD` 只有一项，同样是 `包名/类名`。
 */
class SetupStateTest {

    private val pkg = "com.feihe.ime"

    @Test
    fun `新装 —— 两步都没完成`() {
        // 系统里一个字都没勾，默认还是系统输入法
        val s = SetupState.fromSystem(
            enabledIds = "com.android.inputmethod.latin/.LatinIME",
            defaultId = "com.android.inputmethod.latin/.LatinIME",
            packageName = pkg,
        )
        assertFalse(s.enabled)
        assertFalse(s.selected)
        assertFalse(s.stepEnabledDone)
        assertFalse(s.stepSelectedDone)
        assertFalse(s.allDone)
    }

    @Test
    fun `只启用没切换 —— 第一步完成第二步没做`() {
        // 这是最常见的中间态：用户在系统设置里勾了勾选，然后直接退出设置
        val s = SetupState.fromSystem(
            enabledIds = "com.android.inputmethod.latin/.LatinIME:$pkg/.FeiheIme",
            defaultId = "com.android.inputmethod.latin/.LatinIME",
            packageName = pkg,
        )
        assertTrue("勾选了就该算启用", s.stepEnabledDone)
        assertFalse("没切过来就不算完成第二步", s.stepSelectedDone)
        assertFalse(s.allDone)
    }

    @Test
    fun `两步都完成`() {
        val s = SetupState.fromSystem(
            enabledIds = "$pkg/.FeiheIme:com.android.inputmethod.latin/.LatinIME",
            defaultId = "$pkg/.FeiheIme",
            packageName = pkg,
        )
        assertTrue(s.stepEnabledDone)
        assertTrue(s.stepSelectedDone)
        assertTrue(s.allDone)
    }

    @Test
    fun `做完两步又换回别的输入法 —— 第二步要重新做`() {
        // 用户之前用过飞鹤，临时切回系统输入法打英文，然后忘了切回来
        val s = SetupState.fromSystem(
            enabledIds = "$pkg/.FeiheIme:com.android.inputmethod.latin/.LatinIME",
            defaultId = "com.android.inputmethod.latin/.LatinIME",
            packageName = pkg,
        )
        assertTrue("还是启用状态，不用重新勾", s.stepEnabledDone)
        assertFalse("但当前不是默认，需要重新切换", s.stepSelectedDone)
        assertFalse(s.allDone)
    }

    @Test
    fun `已选中但启用列表里没写 —— 也算已启用`() {
        // 少数定制系统 ENABLED_INPUT_METHODS 的写法不一致，
        // 而系统不允许把未启用的输入法设为默认，所以「已选中」必然意味着已启用。
        // 不加这条兜底会出现「明明正在用飞鹤打字，第一步却显示没做」。
        val s = SetupState.fromSystem(
            enabledIds = null,
            defaultId = "$pkg/.FeiheIme",
            packageName = pkg,
        )
        assertTrue(s.stepEnabledDone)
        assertTrue(s.stepSelectedDone)
        assertTrue(s.allDone)
    }

    @Test
    fun `包名前缀相似的另一个应用 —— 不能误判成我们`() {
        // com.feihe.ime.pro 以 com.feihe.ime 开头，用 contains 会误判
        val s = SetupState.fromSystem(
            enabledIds = "com.feihe.ime.pro/.ProIme",
            defaultId = "com.feihe.ime.pro/.ProIme",
            packageName = pkg,
        )
        assertFalse("别的包名不该算成我们启用", s.enabled)
        assertFalse("别的包名不该算成我们选中", s.selected)
    }

    @Test
    fun `读不到设置 —— 按未完成处理，不崩`() {
        val s = SetupState.fromSystem(enabledIds = null, defaultId = null, packageName = pkg)
        assertFalse(s.stepEnabledDone)
        assertFalse(s.stepSelectedDone)
    }

    @Test
    fun `列表里有空白项 —— 不影响判断`() {
        val s = SetupState.fromSystem(
            enabledIds = "$pkg/.FeiheIme::com.other/.Ime",
            defaultId = "$pkg/.FeiheIme",
            packageName = pkg,
        )
        assertTrue(s.stepEnabledDone)
        assertTrue(s.stepSelectedDone)
    }

    @Test
    fun `很长的类名分号写法也能认`() {
        // 有些输入法在列表里写全限定类名，格式仍是 包名/类名
        val s = SetupState.fromSystem(
            enabledIds = "$pkg/com.feihe.ime.FeiheIme",
            defaultId = "$pkg/com.feihe.ime.FeiheIme",
            packageName = pkg,
        )
        assertTrue(s.stepEnabledDone)
        assertTrue(s.stepSelectedDone)
    }

    @Test
    fun `当前该高亮哪一步`() {
        // 界面靠这两个布尔值决定哪个按钮是蓝的
        val needEnable = SetupState(enabled = false, selected = false)
        assertFalse("第一步没做时第二步不该是主按钮", needEnable.stepEnabledDone)

        val needSwitch = SetupState(enabled = true, selected = false)
        assertTrue(needSwitch.stepEnabledDone)
        assertFalse(needSwitch.stepSelectedDone)

        assertEquals(true, SetupState(enabled = true, selected = true).allDone)
    }
}
