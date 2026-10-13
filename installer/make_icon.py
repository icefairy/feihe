#!/usr/bin/env python3
# 生成飞鹤输入法图标：多尺寸 ICO
# 设计：圆角渐变蓝底 + 白色鹤形/字母 F 意象
import math
from PIL import Image, ImageDraw, ImageFont, ImageFilter

OUT_DIR = "D:/codes/feihe/installer/assets"
import os
os.makedirs(OUT_DIR, exist_ok=True)

def rounded_rect_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return m

def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))

def make_icon(px):
    S = px * 8  # 超采样
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # 对角渐变底：深靛蓝 -> 青蓝
    c1 = (31, 78, 176)     # 深蓝
    c2 = (0, 168, 214)     # 青
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=lerp(c1, c2, t))
    mask = rounded_rect_mask(S, int(S * 0.22))
    img.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(img)

    # 描边高光（顶部白线）
    d.rounded_rectangle([int(S*0.04), int(S*0.04), int(S*0.96), int(S*0.96)],
                        radius=int(S*0.20), outline=(255, 255, 255, 60), width=max(1, S//64))

    # 中央白色「飞」字
    font = None
    for fp in [r"C:\Windows\Fonts\msyhbd.ttc", r"C:\Windows\Fonts\msyh.ttc",
               r"C:\Windows\Fonts\simhei.ttf", r"C:\Windows\Fonts\simsun.ttc"]:
        if os.path.exists(fp):
            try:
                font = ImageFont.truetype(fp, int(S * 0.62))
                break
            except Exception:
                pass
    if font:
        txt = "飞"
        bbox = d.textbbox((0, 0), txt, font=font)
        tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
        # 加一层柔和阴影
        sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        shd = ImageDraw.Draw(sh)
        shd.text((S/2 - tw/2 - bbox[0] + S*0.02, S/2 - th/2 - bbox[1] + S*0.02),
                 txt, font=font, fill=(0, 0, 0, 90))
        sh = sh.filter(ImageFilter.GaussianBlur(S * 0.02))
        img.alpha_composite(sh)
        d = ImageDraw.Draw(img)
        d.text((S/2 - tw/2 - bbox[0], S/2 - th/2 - bbox[1]), txt, font=font,
               fill=(255, 255, 255, 255))
    else:
        # 兜底：画一个简单的鹤喙字母 F
        d.text((S*0.3, S*0.2), "F", fill=(255, 255, 255, 255))

    return img.resize((px, px), Image.LANCZOS)

sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
imgs = [make_icon(s) for s in sizes]
ico_path = f"{OUT_DIR}/feihe.ico"
imgs[-1].save(ico_path, format="ICO", sizes=[(s, s) for s in sizes])
# 也存一个 256 png 便于预览
imgs[-1].save(f"{OUT_DIR}/feihe-256.png")
print("wrote", ico_path, "sizes:", sizes)
