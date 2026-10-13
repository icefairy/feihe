# 飞鹤输入法（feihe）

Windows TSF（Text Services Framework）中文输入法。Rust + windows-rs 实现 COM 组件。

## 里程碑

- **M1（当前）**：TSF 骨架——注册为系统输入法、收键、edit session 上屏（冒烟字符 `飞X`）
- **M2**：librime 引擎接入（全拼候选）
- **M3**：小鹤双拼 schema + 候选窗 UI
- **M4**：词库导入导出（RIME userdict，txt 格式：`词\t编码\t频次`）
- **M5**（v2）：语音输入热键 + ASR + AI 修正（星语 SenseVoice + 璇玑网关）

## 构建（Windows 机）

本机环境走 **llvm-mingw / gnullvm** 路线，**无需 MSVC**（VS 未装 C++ 工具集）。

前置依赖：
- Rust（rustup）
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw/releases) 解压到 `C:\llvm-mingw`（需含顶层 `bin\`，即 `x86_64-w64-mingw32-gcc.exe`）
- `rustup target add x86_64-pc-windows-gnullvm`

一键构建（推荐）：

```powershell
powershell -ExecutionPolicy Bypass -File build.ps1
# 产物：target\x86_64-pc-windows-gnullvm\release\feihe.dll
```

`build.ps1` 会自动：定位 llvm-mingw → 生成 `-lgcc`/`-lgcc_eh` 兼容库（compiler-rt + libunwind）→ 设置 PATH → `cargo build --release`。

> 背景：Rust 的 `*-gnu` target 硬编码链接 `-lgcc`/`-lgcc_eh`，而 llvm-mingw 用 compiler-rt/libunwind，故需 shim 库。
> `rust-toolchain.toml` / `.cargo/config.toml` 已固定 toolchain 与 target，无需手动指定。

原生 MSVC 路线（若日后补装 VS Build Tools C++ 工作负载）：

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc --release
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
build.ps1             一键构建（llvm-mingw/gnullvm）
rust-toolchain.toml   固定 gnu toolchain + gnullvm target
.cargo/config.toml    固定构建 target 与 linker
```

## M1 验证清单

1. `powershell -ExecutionPolicy Bypass -File build.ps1` 成功，生成 `feihe.dll`（gnullvm 路线）
2. register.cmd 后语言栏出现"飞鹤拼音"
3. 切到飞鹤 → 任意输入框敲字母 → 上屏 `飞a` 等冒烟字符（链路证明）
4. unregister.cmd 卸载干净
