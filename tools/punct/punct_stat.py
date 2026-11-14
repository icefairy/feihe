import os, re, sys, collections, glob
import ebooklib
from ebooklib import epub
from html.parser import HTMLParser

class Extract(HTMLParser):
    def __init__(self):
        super().__init__(); self.buf=[]
    def handle_data(self, d): self.buf.append(d)

def epub_text(path):
    out=[]
    try:
        book=epub.read_epub(path, options={'ignore_ncx':True})
        for it in book.get_items():
            if it.get_type()==ebooklib.ITEM_DOCUMENT:
                p=Extract(); p.feed(it.get_content().decode('utf-8','ignore'))
                out.append("".join(p.buf))
    except Exception as e:
        print("ERR epub", path, e, file=sys.stderr)
    return "\n".join(out)

def pdf_text(path):
    import fitz
    out=[]
    try:
        d=fitz.open(path)
        for pg in d: out.append(pg.get_text())
    except Exception as e:
        print("ERR pdf", path, e, file=sys.stderr)
    return "\n".join(out)

files=glob.glob('/data/store/books/*')
texts={}
for f in files:
    ext=f.lower().rsplit('.',1)[-1]
    if ext=='epub': texts[f]=epub_text(f)
    elif ext=='pdf': texts[f]=pdf_text(f)
    print(os.path.basename(f), len(texts.get(f,'')), file=sys.stderr)
import pickle
pickle.dump(texts, open('/tmp/feihe_books_text.pkl','wb'))
print("done", len(texts))
