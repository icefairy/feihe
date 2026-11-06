import io
# 删掉临时调试函数
p = 'core/src/sogou_import.rs'
s = io.open(p, encoding='utf-8').read()
i = s.index('/// 临时调试：把')
j = s.index('}\n', s.index('pub fn debug_scel_words', i)) + 2
s = s[:i] + s[j:]
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)

p2 = 'core/src/sogou.rs'
s2 = io.open(p2, encoding='utf-8').read()
i = s2.index('/// 测试用：暴露 `extract_scel` 的结果')
j = s2.index('}\n', s2.index('pub fn extract_scel_debug', i)) + 2
s2 = s2[:i] + s2[j:]
io.open(p2, 'w', encoding='utf-8', newline='\n').write(s2)
print('ok')
