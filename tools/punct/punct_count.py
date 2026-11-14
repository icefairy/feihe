import pickle, collections, unicodedata
texts=pickle.load(open('/tmp/feihe_books_text.pkl','rb'))
# only chinese-heavy epubs? use all text but remove scanned-empty
alltext="\n".join(texts.values())
tot=len(alltext)
cn_chars=sum(1 for c in alltext if '\u4e00'<=c<='\u9fff')
# punctuation categories
counter=collections.Counter()
for c in alltext:
    cat=unicodedata.category(c)
    if cat.startswith('P') or cat in ('Sm','Sc'):
        counter[c]+=1
print("total chars", tot, "chinese chars", cn_chars)
print(f"{'char':6}{'name':30}{'count':>10}{'per1000cn':>12}")
for c,n in counter.most_common(60):
    try: name=unicodedata.name(c)
    except: name='?'
    print(f"{c!r:6}{name[:28]:30}{n:>10}{n/cn_chars*1000:>12.3f}")
