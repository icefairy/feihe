package com.feihe.ime

/**
 * 「启用 / 切换」这两步的真实状态。
 *
 * ## 为什么要有这个类型，而不是在 Activity 里现查现用
 *
 * 引导页最容易写错的地方不是 UI，是**状态判断**：
 *
 * - 用户可能已经启用但还没切过来（做完第一步，忘了第二步）
 * - 可能切过来了，之后又手动换回别的输入法（两步都做过，但现在没在用）
 * - 可能根本没启用（新装）
 *
 * 如果按「第一次进来显示第一步，点完显示第二步」这种**流程记忆**来做，
 * 上面三种真实状态全都会显示错。用户重进来看见「第一步 启用」已高亮，会以为
 * 自己没启用；或者两步都完成了却还在显示按钮。
 *
 * 所以判据必须每次从**系统真实状态**现算，[fromSystem] 就是这个唯一入口。
 */
data class SetupState(
    /** 系统设置里勾上了我们（``Settings.Secure.ENABLED_INPUT_METHODS`` 含本包） */
    val enabled: Boolean,
    /** 当前默认输入法就是我们（``Settings.Secure.DEFAULT_INPUT_METHOD`` 是本包） */
    val selected: Boolean,
) {
    /**
     * 第一步「启用」是否已经完成。
     *
     * 注意：**已选中必然意味着已启用**（系统不允许把未启用的输入法设为默认）。
     * 但反过来不成立。有些设备上 `ENABLED_INPUT_METHODS` 的写法有差异，
     * 所以把「已选中」也算作「已启用」，避免出现「明明在打字，第一步却显示没做」。
     */
    val stepEnabledDone: Boolean get() = enabled || selected

    /** 第二步「切换」是否已经完成 */
    val stepSelectedDone: Boolean get() = selected

    /** 两步都好了：不需要再引导 */
    val allDone: Boolean get() = stepEnabledDone && stepSelectedDone

    companion object {
        /**
         * 从系统读数算出状态。
         *
         * @param enabledIds `Settings.Secure.ENABLED_INPUT_METHODS` 的原始字符串，
         *   形如 `com.feihe.ime/.FeiheIme:com.android.inputmethod/.LatinIME`，
         *   每项是 `包名/类名`，用 `:` 分隔。允许为 null（读不到时按未启用处理）。
         * @param defaultId `Settings.Secure.DEFAULT_INPUT_METHOD` 的原始字符串，
         *   形如 `com.feihe.ime/.FeiheIme`。
         * @param packageName 本应用包名
         */
        fun fromSystem(enabledIds: String?, defaultId: String?, packageName: String): SetupState {
            // 用 startsWith("包名/") 而不是 contains(包名)：包名可能是另一个包名的前缀
            // （com.feihe.ime 和 com.feihe.ime.pro），contains 会误判。
            fun matches(entry: String): Boolean {
                val e = entry.trim()
                return e == packageName || e.startsWith("$packageName/")
            }

            val enabled = enabledIds
                ?.split(':')
                ?.any { it.isNotBlank() && matches(it) }
                ?: false
            val selected = defaultId?.let { matches(it) } ?: false
            return SetupState(enabled = enabled, selected = selected)
        }
    }
}
