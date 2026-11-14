import pickle, collections, unicodedata
texts=pickle.load(open('/tmp/feihe_books_text.pkl','rb'))
# 只用中文比例高的书（epub 为主，排除扫描空 pdf）
picked={}
for f,t in texts.items():
    cn=sum(1 for c in t if '\u4e00'<=c<='\u9fff')
    if cn>50000:
        picked[f]=t
alltext="\n".join(picked.values())
cn=sum(1 for c in alltext if '\u4e00'<=c<='\u9fff')
cnt=collections.Counter()
for c in alltext:
    if unicodedata.category(c).startswith('P'):
        cnt[c]+=1
total_p=sum(cnt.values())
print("纳入书籍:", len(picked), " 汉字总数:", cn, " 标点总数:", total_p)
print("%-6s %-28s %10s %10s %8s"%('字符','名称','次数','每千汉字','占标点%'))
for c,n in cnt.most_common(30):
    if n<30: continue
    try: name=unicodedata.name(c)
    except: name='?'
    print("%-6s %-28s %10d %10.2f %7.2f%%"%(c,name[:26],n,n/cn*1000,n/total_p*100))
