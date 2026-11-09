#!/usr/bin/env python3
"""从 Windows 那套图标生成 Android 启动图标。

设计只有一份：深蓝→青的竖向渐变圆角方块 + 白色「飞」字。
Windows 侧由 `installer/make_icon.py` 产出 .ico，这里产出 Android 要的三种：

  mipmap-*/ic_launcher.png          传统图标（48 / 72 / 96 / 144 / 192）
  mipmap-*/ic_launcher_round.png    圆形版本（部分启动器按圆形裁）
  mipmap-anydpi-v26/ic_launcher.xml 自适应图标（API 26+）的装配文件
  mipmap-xxxhdpi/ic_launcher_foreground.png / _background.png  它的两层

**颜色和字形照抄 Windows 那份**：底 c1=(31,78,176)、c2=(0,168,214)，字「飞」，
字体 msyhbd。两端图标不一样的话，装完看着像两个软件。

自适应图标的两层有讲究：背景必须**满幅**（启动器会按自己的形状裁），前景里的
字形只能占中间 66/108 的安全区，否则被裁掉一圈。
"""
import os
from PIL import Image, ImageDraw, ImageFont, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, "android", "app", "src", "main", "res")

C1 = (31, 78, 176)
C2 = (0, 168, 214)
FONT_CANDIDATES = [
    r"C:\Windows\Fonts\msyhbd.ttc",
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\simhei.ttf",
]

DENSITIES = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}
ADAPTIVE_PX = 432  # 108dp @ xxxhdpi


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def gradient(size):
    grad = Image.new("RGBA", (size, size))
    d = ImageDraw.Draw(grad)
    for y in range(size):
        d.line([(0, y), (size, y)], fill=lerp(C1, C2, y / max(1, size - 1)))
    return grad


def load_font(px):
    for fp in FONT_CANDIDATES:
        if os.path.exists(fp):
            try:
                return ImageFont.truetype(fp, px)
            except Exception:
                pass
    return None


def draw_glyph(img, size, ratio, with_shadow=True):
    """在白底上居中画「飞」。ratio = 字高占画布的比例。"""
    d = ImageDraw.Draw(img)
    font = load_font(int(size * ratio))
    if font is None:
        return img
    txt = "飞"
    bbox = d.textbbox((0, 0), txt, font=font)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    x = size / 2 - tw / 2 - bbox[0]
    y = size / 2 - th / 2 - bbox[1]
    if with_shadow:
        sh = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        ImageDraw.Draw(sh).text((x + size * 0.02, y + size * 0.02), txt, font=font, fill=(0, 0, 0, 90))
        img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(size * 0.02)))
        d = ImageDraw.Draw(img)
    d.text((x, y), txt, font=font, fill=(255, 255, 255, 255))
    return img


def masked(size, shape):
    """满幅渐变 + 形状蒙版，得到传统图标。"""
    s = size * 8  # 超采样，边缘才干净
    img = gradient(s)
    m = Image.new("L", (s, s), 0)
    md = ImageDraw.Draw(m)
    if shape == "round":
        md.ellipse([0, 0, s - 1, s - 1], fill=255)
    else:
        md.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.22), fill=255)
    out = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    out.paste(img, (0, 0), m)
    # Windows 那份的描边高光
    d = ImageDraw.Draw(out)
    d.rounded_rectangle(
        [int(s * 0.04), int(s * 0.04), int(s * 0.96), int(s * 0.96)],
        radius=int(s * 0.20),
        outline=(255, 255, 255, 60) if shape != "round" else (0, 0, 0, 0),
        width=max(1, s // 64),
    )
    draw_glyph(out, s, 0.62)
    return out.resize((size, size), Image.LANCZOS)


def foreground(size=ADAPTIVE_PX):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    # 安全区 66/108 ≈ 61%，字再小一点留白，别顶到裁切线
    draw_glyph(img, size, 0.46, with_shadow=False)
    return img


def main():
    for dpi, px in DENSITIES.items():
        d = os.path.join(RES, "mipmap-" + dpi)
        os.makedirs(d, exist_ok=True)
        masked(px, "square").save(os.path.join(d, "ic_launcher.png"))
        masked(px, "round").save(os.path.join(d, "ic_launcher_round.png"))
        print("wrote", d)

    # 自适应图标的两层放 xxxhdpi：系统会按需缩放，没必要每个密度都存一份
    d = os.path.join(RES, "mipmap-xxxhdpi")
    os.makedirs(d, exist_ok=True)
    gradient(ADAPTIVE_PX).save(os.path.join(d, "ic_launcher_background.png"))
    foreground().save(os.path.join(d, "ic_launcher_foreground.png"))

    anydpi = os.path.join(RES, "mipmap-anydpi-v26")
    os.makedirs(anydpi, exist_ok=True)
    xml = """<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"""
    for name in ("ic_launcher.xml", "ic_launcher_round.xml"):
        with open(os.path.join(anydpi, name), "w", encoding="utf-8") as f:
            f.write(xml)
    print("wrote adaptive icon xml")


if __name__ == "__main__":
    main()
