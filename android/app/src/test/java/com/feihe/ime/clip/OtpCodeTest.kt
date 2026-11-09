package com.feihe.ime.clip

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OtpCodeTest {

    /** 整条就是验证码——用户手动复制的常见形态。 */
    @Test
    fun whole_text_is_code() {
        assertEquals("123456", OtpCode.extract("123456"))
    }

    /** 短信原文：关键词后面那个数字。 */
    @Test
    fun extracts_code_after_keyword() {
        assertEquals("123456", OtpCode.extract("【飞鹤】验证码123456，5分钟内有效。"))
        assertEquals("8891", OtpCode.extract("您的验证码为 8891"))
    }

    /** 全角数字要能认出来，否则中文短信里那一份经常漏。 */
    @Test
    fun full_width_digits_are_normalized() {
        assertEquals("123456", OtpCode.extract("验证码：１２３４５６"))
    }

    /** 没有关键词、整段很短且只有一个数字串 → 认它。 */
    @Test
    fun short_text_single_number_without_keyword() {
        assertEquals("654321", OtpCode.extract("654321"))
        assertEquals("482913", OtpCode.extract("登录代码 482913"))
    }

    /** 订单号太长（13 位）不是验证码。 */
    @Test
    fun long_order_number_is_not_a_code() {
        assertNull(OtpCode.extract("订单号 2023110912345"))
    }

    /** 年份不能被当成验证码。 */
    @Test
    fun year_is_not_a_code() {
        assertNull(OtpCode.extract("有效期至 2023 年"))
    }

    /** 长文本里出现一个数字不算——那是文章，不是验证码。 */
    @Test
    fun long_text_without_keyword_is_ignored() {
        val text = "今天的会议纪要已经发到群里了，大家看一下第三项关于排期的部分，有问题明天讨论。" +
            "另外 1234 号会议室改到下午。"
        assertNull(OtpCode.extract(text))
    }

    /** 数字框 / 电话框直接算验证码框。 */
    @Test
    fun numeric_field_is_otp_field() {
        assertTrue(OtpCode.looksLikeOtpField(0x00000002, null)) // TYPE_CLASS_NUMBER
        assertTrue(OtpCode.looksLikeOtpField(0x00000003, null)) // TYPE_CLASS_PHONE
        assertFalse(OtpCode.looksLikeOtpField(0x00000001, null)) // TYPE_CLASS_TEXT
    }

    /** 文本框靠 hint 认。 */
    @Test
    fun hint_keyword_marks_otp_field() {
        assertTrue(OtpCode.looksLikeOtpField(0x00000001, "请输入验证码"))
        assertFalse(OtpCode.looksLikeOtpField(0x00000001, "搜索"))
    }

    /** 关键词判断单独可用（正文里带「验证码」时兜底）。 */
    @Test
    fun keyword_detection() {
        assertTrue(OtpCode.hasKeyword("Your verification code is 112233"))
        assertFalse(OtpCode.hasKeyword("今天天气不错"))
    }
}
