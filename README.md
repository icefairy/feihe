# 飞鹤输入法（feihe）

Windows TSF（Text Services Framework）中文输入法。Rust + windows-rs 实现 COM 组件。

## 里程碑

- **M1（当前）**：TSF 骨架——注册为系统输入法、收键、edit session 上屏（冒烟字符 `飞X`）
- **M2**：librime 引擎接入（全拼候选）
- **M3**：小鹤双拼 schema + 候选窗 UI
- **M4**：词库导入导出（RIME userdict，txt 格式：`词\t编码\t频次`）
- **M5**（v2）：语音输入热键 + ASR + AI 修正（星语 SenseVoice + 璇玑网关）

## 构建（Windows 机）

```powershell
# 依赖：Rust（rustup 默认 msvc toolchain）+ VS Build Tools (C++ 工作负载)
git clone <repo>
cd feihe
cargo build --target x86_64-pc-windows-msvc --release
# 产物：target\x86_64-pc-windows-msvc\release\feihe.dll
```

## 安装 / 卸载（管理员 cmd）

```cmd
cd installer && register.cmd      :: 注册（写入 CTF\TIP）
:: 到 设置-时间和语言-语言-中文-选项-键盘 里出现"飞鹤拼音"，启用它
unregister.cmd                    :: 卸载
```

## 目录

```
src/lib.rs            DLL 入口（DllGetClassObject）
src/clsid.rs          固定 GUID
src/classfactory.rs   IClassFactory
src/textservice.rs    TSF 核心（Activate/按键/上屏）
installer/            注册脚本
```

## M1 验证清单

1. `cargo build --target x86_64-pc-windows-msvc --release` 成功
2. register.cmd 后语言栏出现"飞鹤拼音"
3. 切到飞鹤 → 任意输入框敲字母 → 上屏 `飞a` 等冒烟字符（链路证明）
4. unregister.cmd 卸载干净
