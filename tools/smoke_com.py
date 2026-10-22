"""COM 冒烟测试：验证 feihe.dll 能被加载、类厂能创建对象、引用计数能归零。

不涉及 TSF 上下文，只测 COM 骨架。任何一步失败都说明注册或导出有问题。
"""

import ctypes
import ctypes.wintypes as wt
import sys

DLL = r"C:\ProgramData\Feihe\feihe.dll"

# {70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
CLSID_FEIHE = "{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}"
IID_ICLASSFACTORY = "{00000001-0000-0000-C000-000000000046}"
IID_ITFTEXTINPUTPROCESSOR = "{AA80E7F7-2021-11D2-93E0-0060B067B86E}"

S_OK = 0
S_FALSE = 1


class GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", wt.DWORD),
        ("Data2", wt.WORD),
        ("Data3", wt.WORD),
        ("Data4", ctypes.c_ubyte * 8),
    ]

    def __init__(self, s):
        super().__init__()
        s = s.strip("{}")
        p = s.split("-")
        self.Data1 = int(p[0], 16)
        self.Data2 = int(p[1], 16)
        self.Data3 = int(p[2], 16)
        rest = p[3] + p[4]
        for i in range(8):
            self.Data4[i] = int(rest[i * 2 : i * 2 + 2], 16)


def hr(ok):
    return "OK" if ok else "FAIL"


def check(label, cond):
    print(f"  [{hr(cond)}] {label}")
    if not cond:
        sys.exit(1)


def main():
    print(f"loading {DLL}")
    dll = ctypes.WinDLL(DLL)

    dll.DllGetClassObject.argtypes = [
        ctypes.POINTER(GUID),
        ctypes.POINTER(GUID),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    dll.DllGetClassObject.restype = ctypes.c_long
    dll.DllCanUnloadNow.argtypes = []
    dll.DllCanUnloadNow.restype = ctypes.c_long

    clsid = GUID(CLSID_FEIHE)
    iid_cf = GUID(IID_ICLASSFACTORY)
    iid_tip = GUID(IID_ITFTEXTINPUTPROCESSOR)

    print("1. DllCanUnloadNow (before any object)")
    rc = dll.DllCanUnloadNow()
    check(f"returns S_OK={S_OK}, got {rc}", rc == S_OK)

    print("2. DllGetClassObject -> IClassFactory")
    pcf = ctypes.c_void_p()
    rc = dll.DllGetClassObject(ctypes.byref(clsid), ctypes.byref(iid_cf), ctypes.byref(pcf))
    check(f"HRESULT 0x{rc & 0xFFFFFFFF:08X}", rc == S_OK and pcf.value)

    # IClassFactory vtable: IUnknown(3) + CreateInstance + LockServer
    vtbl = ctypes.cast(pcf, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
    CreateInstance = ctypes.WINFUNCTYPE(
        ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(GUID), ctypes.POINTER(ctypes.c_void_p)
    )(vtbl[3])
    LockServer = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, wt.BOOL)(vtbl[4])
    ReleaseCF = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(vtbl[2])

    print("3. CreateInstance -> ITfTextInputProcessor")
    ptip = ctypes.c_void_p()
    rc = CreateInstance(pcf, None, ctypes.byref(iid_tip), ctypes.byref(ptip))
    check(f"HRESULT 0x{rc & 0xFFFFFFFF:08X}", rc == S_OK and ptip.value)

    print("4. DllCanUnloadNow (object alive)")
    rc = dll.DllCanUnloadNow()
    check(f"returns S_FALSE={S_FALSE}, got {rc}", rc == S_FALSE)

    print("5. LockServer(TRUE) then DllCanUnloadNow")
    rc = LockServer(pcf, True)
    check("LockServer succeeded", rc == S_OK)
    rc = dll.DllCanUnloadNow()
    check(f"returns S_FALSE={S_FALSE}, got {rc}", rc == S_FALSE)
    LockServer(pcf, False)

    print("6. release text service object")
    tip_vtbl = ctypes.cast(ptip, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
    ReleaseTIP = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(tip_vtbl[2])
    n = ReleaseTIP(ptip)
    check(f"refcount dropped to {n}", n == 0)

    print("7. DllCanUnloadNow (class factory still alive)")
    # 类工厂本身也算一个 COM 对象，所以此时仍应是 S_FALSE
    rc = dll.DllCanUnloadNow()
    check(f"returns S_FALSE={S_FALSE}, got {rc}", rc == S_FALSE)

    print("8. release class factory")
    ReleaseCF(pcf)

    print("9. DllCanUnloadNow (everything released)")
    rc = dll.DllCanUnloadNow()
    check(f"returns S_OK={S_OK}, got {rc}", rc == S_OK)

    print("\nall checks passed")


if __name__ == "__main__":
    main()
