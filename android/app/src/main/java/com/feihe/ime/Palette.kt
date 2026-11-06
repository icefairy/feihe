package com.feihe.ime

import android.content.Context
import android.content.res.Configuration

/**
 * 全 App 的配色表：**浅色 / 深色两套**，按系统的夜间模式自动选。
 *
 * ## 为什么是一张表而不是 `values-night/colors.xml`
 *
 * 这个项目里所有界面都是**程序化构建**的（`KeyboardView.build` / `SettingsActivity.buildUi`
 * 全用 Kotlin 建 View，没有 XML 布局）。资源里放一套 colors，代码还是得 `getColor()` 挨个取，
 * 等于多一层间接；而"改一个颜色要动几处"这个问题的答案是**收成一张表**，与放哪无关。
 *
 * 所以：颜色只在这个文件里定义，其它文件一律 `Palette.of(ctx).xxx`。
 * 新加颜色时**两个实例都要给值**——漏给一个，深色下就是一片白底白字。
 *
 * ## 什么时候会重新取
 *
 * `of()` 每次读 `resources.configuration.uiMode`，代价是几十纳秒，不值得缓存：
 * 系统切换深色模式时资源会重建，缓存反而要处理失效。键盘重建（换页/换方案）本来
 * 就不频繁，候选条的每格重绑也不在热路径上。
 */
object Palette {

    /**
     * 一套完整配色。字段按**用途**命名，不按"某某控件"命名——
     * 同一个 `SUB` 要同时喂设置页说明文字、浮窗次要信息、联想项，
     * 按控件命名会立刻分裂成五个近似值。
     */
    data class P(
        /** 页面底色（设置页根布局） */
        val bg: Int,
        /** 卡片 / 候选条底色 */
        val card: Int,
        /** 主文字 */
        val text: Int,
        /** 次要文字（说明、提示、联想项） */
        val sub: Int,
        /** 强调色（第一个候选、选中态、开关点亮） */
        val accent: Int,
        /** 强调色上面的文字（点亮的上档键） */
        val onAccent: Int,
        /** 字母 / 数字 / 标点键的键帽底色 */
        val keyBg: Int,
        /** 键帽上的字 */
        val keyText: Int,
        /** 空格键：比字母键略暗一点，一眼分出"这不是一个字" */
        val spaceBg: Int,
        /** 功能键（⇧ ⌫ ↵ ABC 符 …）底色 */
        val fnBg: Int,
        /** 功能键文字 */
        val fnText: Int,
        /** 数字选词行：淡强调色底，和"可打的字"区分开 */
        val selectBg: Int,
        /** 键盘整体底色（键与键之间的缝） */
        val keyboardBg: Int,
        /** 键帽描边 */
        val stroke: Int,
        /** 状态条底色 */
        val statusBg: Int,
        /** 候选与联想之间的分隔竖线 */
        val separator: Int,
        /** 分段 chip 的描边 */
        val chipStroke: Int,
        /** 更淡的提示（「↵上屏」这类） */
        val hint: Int,
        /** 错误红（引擎未就绪） */
        val danger: Int,
    )

    /** 浅色。这一套是原来散落各处的字面值，数值一个都没动。 */
    private val LIGHT = P(
        bg = 0xFFF2F3F5.toInt(),
        card = 0xFFFFFFFF.toInt(),
        text = 0xFF202124.toInt(),
        sub = 0xFF5F6368.toInt(),
        accent = 0xFF1A6FD4.toInt(),
        onAccent = 0xFFFFFFFF.toInt(),
        keyBg = 0xFFFFFFFF.toInt(),
        keyText = 0xFF000000.toInt(),
        spaceBg = 0xFFF7F7F8.toInt(),
        fnBg = 0xFFB9BEC6.toInt(),
        fnText = 0xFF333333.toInt(),
        selectBg = 0xFFE8F0FE.toInt(),
        keyboardBg = 0xFFD9DCE1.toInt(),
        stroke = 0xFF9AA0A8.toInt(),
        statusBg = 0xFFEDEEF0.toInt(),
        separator = 0xFFBDBDBD.toInt(),
        chipStroke = 0xFFD9DCE1.toInt(),
        hint = 0xFF9AA0A8.toInt(),
        danger = 0xFFC5221F.toInt(),
    )

    /**
     * 深色。
     *
     * 三条取值原则，改颜色时照着来：
     * 1. **不用纯黑**（`#000`）当底色：OLED 上滑动会出现拖影，且和系统其它应用放一起
     *    显得突兀。用 `#121212` 这一档。
     * 2. **键帽比底色亮一档**，靠明度分出"能按的地方"，而不是靠描边——深色下细描边
     *    几乎看不见，键会糊成一片。
     * 3. **强调色提亮**：浅色用的 `#1A6FD4` 放在深色底上对比度不够（蓝字发灰）。
     *    `#8AB4F8` 是 Material 深色主题对蓝色强调色的推荐档，白字/深字都能压住。
     */
    private val DARK = P(
        bg = 0xFF121212.toInt(),
        card = 0xFF1E1F22.toInt(),
        text = 0xFFE8EAED.toInt(),
        sub = 0xFF9AA0A6.toInt(),
        accent = 0xFF8AB4F8.toInt(),
        onAccent = 0xFF121212.toInt(),
        keyBg = 0xFF33363B.toInt(),
        keyText = 0xFFE8EAED.toInt(),
        spaceBg = 0xFF2C2F33.toInt(),
        fnBg = 0xFF4A4E55.toInt(),
        fnText = 0xFFE8EAED.toInt(),
        selectBg = 0xFF1F3A5F.toInt(),
        keyboardBg = 0xFF0F1012.toInt(),
        stroke = 0xFF15171A.toInt(),
        statusBg = 0xFF1B1C1E.toInt(),
        separator = 0xFF5F6368.toInt(),
        chipStroke = 0xFF3A3D42.toInt(),
        hint = 0xFF80858C.toInt(),
        danger = 0xFFFF897D.toInt(),
    )

    /** 当前该用哪一套。 */
    fun of(ctx: Context): P {
        val night = (ctx.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES
        return if (night) DARK else LIGHT
    }
}
