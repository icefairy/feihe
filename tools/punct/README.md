# punct —— 标点频率统计工具

给「标点布局重排」提供数据依据。见 `docs/标点布局-频次依据与重排方案.md`。

## 依赖
python3 + PyMuPDF(fitz) + ebooklib
```
pip install pymupdf ebooklib --quiet
```

## 用法
```
# 1) 抽取 /data/store/books 下所有 pdf/epub 文本 -> /tmp/feihe_books_text.pkl
python3 punct_stat.py

# 2) 全量标点计数（含低质扫描 pdf，仅供粗看）
python3 punct_count.py

# 3) 【推荐】只统计汉字>5万的高质量中文书，输出中文标点频率表
python3 cn_stat.py
```

## 说明
- 度量：每千汉字（‰）与占全部标点百分比（%）。
- 书源路径写死为 `/data/store/books`；换目录改 `punct_stat.py` 顶部 glob。
- 电子书偏文学/医案，`！？…` 会偏高，但**相对排序**可靠。
