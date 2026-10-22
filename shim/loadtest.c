/* loadtest.c — 32 位 DLL 冒烟测试（不进仓库产物，只在 build 校验时用）
 *
 * 做的事：
 *   1. LoadLibrary 目标 DLL —— 验证依赖齐全、DllMain 能跑通
 *   2. GetProcAddress 拿 DllGetClassObject / DllCanUnloadNow
 *   3. 调 DllGetClassObject 取 IClassFactory —— 验证 CLSID 映射与 COM 骨架
 *
 * 退出码：0 全通过；非 0 表示失败（并打印 Win32 错误码）。
 */
/* COBJMACROS: C 模式下 IClassFactory / IUnknown 只有 vtable，没有同名成员函数，
 * 打开它才有 IClassFactory_CreateInstance / IUnknown_Release 这些便捷宏。 */
#define COBJMACROS
#include <windows.h>
#include <stdio.h>
#include <objbase.h>

int main(int argc, char **argv) {
    if (argc < 2) { printf("usage: loadtest.exe <dll> [clsid]\n"); return 2; }

    HMODULE h = LoadLibraryA(argv[1]);
    if (!h) {
        printf("FAIL LoadLibrary err=%lu\n", GetLastError());
        return 1;
    }
    printf("OK   LoadLibrary %s\n", argv[1]);

    FARPROC gco = GetProcAddress(h, "DllGetClassObject");
    FARPROC cun = GetProcAddress(h, "DllCanUnloadNow");
    printf("OK   DllGetClassObject=%p DllCanUnloadNow=%p\n", (void *)gco, (void *)cun);
    if (!gco || !cun) { FreeLibrary(h); return 1; }

    if (argc >= 3) {
        /* {70CB823D-D08A-4B57-B8BE-08BCE2539FF3} */
        CLSID clsid;
        if (FAILED(CLSIDFromString(L"{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}", &clsid))) {
            printf("FAIL CLSIDFromString\n"); FreeLibrary(h); return 1;
        }
        IClassFactory *cf = NULL;
        HRESULT hr = ((HRESULT (STDAPICALLTYPE *)(REFCLSID, REFIID, void **))gco)(
            &clsid, &IID_IClassFactory, (void **)&cf);
        if (FAILED(hr)) {
            printf("FAIL DllGetClassObject hr=0x%08lX\n", (unsigned long)hr);
            FreeLibrary(h); return 1;
        }
        printf("OK   DllGetClassObject -> IClassFactory %p\n", (void *)cf);

        IUnknown *unk = NULL;
        hr = IClassFactory_CreateInstance(cf, NULL, &IID_IUnknown, (void **)&unk);
        printf("     CreateInstance hr=0x%08lX ptr=%p\n", (unsigned long)hr, (void *)unk);
        if (unk) IUnknown_Release(unk);
        IUnknown_Release(cf);
    }

    FreeLibrary(h);
    printf("PASS\n");
    return 0;
}
