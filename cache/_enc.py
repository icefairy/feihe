# -*- coding: utf-8 -*-
import re
src = open('core/src/shuangpin.rs', encoding='utf-8').read()
m = re.search(r'const XIAOHE_FINALS: &\[\(&str, char\)\] = &\[(.*?)\];', src, re.S)
finals = dict(re.findall(r'\("([^"]+)", \'(.)\'\)', m.group(1)))
INIT = 'bpmfdtnlgkhjqxrzcsyw'

def final_key(fin):
    if fin in finals:
        return finals[fin]
    return None

def enc_syl(s):
    if s[:2] in ('zh', 'ch', 'sh') and len(s) >= 3:
        ini = {'zh': 'v', 'ch': 'i', 'sh': 'u'}[s[:2]]
        rest = s[2:]
        k = final_key(rest)
        return None if k is None else ini + k
    if s[0] in INIT:
        rest = s[1:]
        k = final_key(rest)
        return None if k is None else s[0] + k
    # zero initial
    if len(s) == 1:
        return finals.get(s, None) and s + finals[s]
    if s in ('ai', 'ei', 'ao', 'ou', 'an', 'en', 'er'):
        return s  # 零声母 + 末字母 == 全拼本身
    k = final_key(s)
    return None if k is None else (s[0] + k) if k != s[0] else s[0] * 2

def enc(py):
    out = ''
    for syl in py.split():
        e = enc_syl(syl)
        if e is None:
            return '??' + syl
        out += e
    return out

import sys
for w, py in [(a, b) for a, b in (x.split('|') for x in sys.argv[1:])]:
    print(w, py, '->', enc(py))
