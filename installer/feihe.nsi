; feihe.nsi — 飞鹤拼音 安装程序（NSIS 3.x / Unicode）
;
; 设计取舍
; --------
; 1. **注册逻辑不在这里重写**。COM / CTF TIP / 用户键盘列表这三件事全部交给
;    install_tip.ps1、uninstall_tip.ps1 —— 它们是唯一真相，开发时 register.cmd
;    用的也是同一份。安装包只负责「提权 + 拷文件 + 调脚本 + 卸载时反向调」。
;    否则 Pascal 里再来一套，两份实现必然漂移。
; 2. 装到 Program Files（不是老的 C:\ProgramData\Feihe）。安装包该有安装包的样子，
;    老的 ProgramData 副本会在安装时尽力清理（删不掉的留着，不安排重启删除）。
; 3. 覆盖 DLL 时先 Delete 再 Rename 兜底：ctfmon / explorer 常常已经把旧 DLL
;    映射进进程，直接 Copy 会失败。Windows 允许**重命名**已映射的文件，
;    所以改名成 .old-<随机> 再拷新文件就行。后缀必须唯一——固定 `.old` 一旦被
;    上一次留下的、仍被映射的旧文件占着，删不掉也改不进去，整条替换链就断了
;    （这个坑真的发生过：装完仍是上一版 DLL，功能像没修一样）。
;    但改名只换了磁盘上的文件——ctfmon 进程里映射的仍是旧代码。install_tip.ps1
;    最后一步会重启 ctfmon（Windows 按需自动拉起，中等完整性；不能从提权进程
;    手动拉，UIPI 会挡消息），覆盖安装后点一下输入框就加载新 DLL，不用注销。
;    卸载侧对称：uninstall_tip.ps1 末尾同样重启，之后 Delete 才删得掉被映射的 DLL。
;
; 编译：makensis /DARCH=x64 feihe.nsi   （或直接跑 build_setup.ps1）

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; ── 基本信息 ────────────────────────────────────────────────────
!define APP_NAME      "飞鹤拼音"
!define APP_NAME_EN   "Feihe IME"
!define APP_VER       "0.3.0"
!define APP_PUBLISHER "feihe"
!define APP_URL       "https://github.com/icefairy/feihe"

!define CLSID    "{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}"
!define PROFILE  "{DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}"
!define KBD_GUID "{34745C63-B2F0-4784-8B67-5E12C8701A31}"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FeiheIME"

; ── 产物路径（由 build_setup.ps1 通过 /DOUTDIR= 传入）────────────
!ifndef OUTDIR
  !define OUTDIR "..\dist"
!endif
!ifndef SRCDIR
  !define SRCDIR "..\target"
!endif

; 32 位走 gnullvm（和 64 位同一条工具链），产物目录不同
!define DLL64 "${SRCDIR}\x86_64-pc-windows-gnullvm\release\feihe.dll"
!define DLL32 "${SRCDIR}\i686-pc-windows-gnullvm\release\feihe.dll"

; 用户手册（Markdown）。装到安装目录，用户不用去 GitHub 找。
!define HELPDOC "..\docs\使用帮助.md"

; ── 编译器属性 ──────────────────────────────────────────────────
;
; **默认装到 `%LOCALAPPDATA%\Feihe`，用户级、不要管理员。**
;
; 为什么不是 `Program Files`：输入法的 DLL 跑在记事本 / Word / 浏览器这些普通
; 权限进程里，它们写不了 `Program Files`。而本版本起 config.ini / freq.txt /
; phrases.txt / pinned.txt 全都放在安装目录，目标是「压缩一份目录即备份」——
; 装到 `Program Files` 的话，写入会被 UAC 重定向到 VirtualStore（32 位宿主）或者
; 直接失败（64 位宿主），备份就成了假象，而且「有时能存有时不能存」最难查。
;
; 用户级安装的连带好处：不需要 UAC，注册表全在 HKCU（含 CTF TIP），
; 卸载不需要提权，多用户各装各的互不干扰。
;
; `SELFTEST` 由 build_setup.ps1 -SelfTest 传入：装到临时目录，验证「真装一遍
; 再比对 SHA256」而不碰用户真实安装。
!ifdef SELFTEST
  !define INSTALL_ROOT "$TEMP\feihe-selftest"
!else
  !define INSTALL_ROOT "$LOCALAPPDATA\Feihe"
!endif
!define REQ_LEVEL "user"

Name "${APP_NAME}"
OutFile "${OUTDIR}\Feihe-Setup-${APP_VER}.exe"
InstallDir "${INSTALL_ROOT}"
; 自检模式不读注册表里的旧安装路径，否则会被本机真实安装覆盖。
; 正式包优先读 HKCU，其次读旧版写下的 HKLM（从 Program Files 升级过来的用户）。
!ifndef SELFTEST
  InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
!endif
RequestExecutionLevel "${REQ_LEVEL}"
SetCompressor /SOLID lzma
SetCompressorDictSize 32
ShowInstDetails show
ShowUninstDetails show
BrandingText "${APP_NAME} ${APP_VER} — 小鹤双拼 / 全拼 输入法"

VIProductVersion "0.3.0.0"
VIAddVersionKey /LANG=2052 "ProductName"     "${APP_NAME}"
VIAddVersionKey /LANG=2052 "FileDescription" "${APP_NAME} 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion"     "${APP_VER}"
VIAddVersionKey /LANG=2052 "ProductVersion"  "${APP_VER}"
VIAddVersionKey /LANG=2052 "CompanyName"     "${APP_PUBLISHER}"
VIAddVersionKey /LANG=2052 "LegalCopyright"  "MIT License"

Icon "assets\feihe.ico"
UninstallIcon "assets\feihe.ico"

; ── 界面 ────────────────────────────────────────────────────────
!define MUI_ABORTWARNING
!define MUI_ICON "assets\feihe.ico"
!define MUI_UNICON "assets\feihe.ico"
!define MUI_FINISHPAGE_NOAUTOCLOSE
; 完成页绝不提供「重启计算机」。本安装根本不需要重启电脑：DLL 是改名替换的，
; 生效靠 install_tip.ps1 末尾重启 **ctfmon 进程**，跟系统重启无关。以前这里没管，
; 只要有一条路径调了 Delete /REBOOTOK（旧副本删不掉就会），NSIS 就会置重启标记，
; 完成页立刻多出一个「重启计算机」，用户看到就以为「必须重启」。
; 这个宏是最后一道保险：就算别处再置标记，完成页也不会显示重启选项。
!define MUI_FINISHPAGE_NOREBOOTSUPPORT
!define MUI_FINISHPAGE_TEXT "${APP_NAME} 已安装。$\r$\n$\r$\n用 Win+Space 或 Ctrl+Shift 切换到「${APP_NAME}」。$\r$\n$\r$\n如果输入法列表里还看不到它，请注销后重新登录（列表由 shell 缓存）。不需要重启电脑。"
!define MUI_UNFINISHPAGE_NOAUTOCLOSE

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; ── 工具函数 ────────────────────────────────────────────────────

; 覆盖一个可能被占用（已被 ctfmon / explorer / 记事本映射）的文件。
; 入参：$R8 = 源文件，$R9 = 目标文件；出参：失败时置 $R5 = 1
;
; 失败链是踩出来的，改之前先看完：
;   1. 直接 Copy 失败——目标被映射，正常现象
;   2. 改名成**固定** `.old`——如果上一次留下的 `.old` 还被旧进程映射着，
;      `Delete` 删不掉、`Rename` 又因为目标已存在而失败，整条链就断在这里
;   3. 再 Copy 还是失败 → 老版本走 `Delete /REBOOTOK "$R9"`，那是**删掉**，
;      用户一重启输入法就彻底没了
;   4. 兜底校验只看 `FileExists`，旧文件还在 → 静默「安装成功」
;      （真实案例：装完 feihe.dll 还是上一个版本，右键置顶、排序修复全都不生效）
; 所以现在：改名一律用**唯一后缀**（保证 Rename 的目标不存在）；
;           不再做 /REBOOTOK 删除（宁可当场报错，让人关掉程序重装）；
;           复制完比**文件大小**，不一致算失败。
Function CopyOrReplace
  ClearErrors
  CopyFiles /SILENT "$R8" "$R9"
  IfErrors 0 verify

  DetailPrint "  $R9 被占用，改名旧文件后重试"
  ; 后缀要唯一：固定名一旦被上一次的 .old 占着，Rename 就失败了。
  ; NSIS 没有 $RANDOM（老版本没有），用 GetTickCount 的毫秒数当随机源。
  System::Call 'kernel32::GetTickCount()i.r2'
  StrCpy $R2 "$R9.old-$R2"
  ${If} ${FileExists} "$R2"
    StrCpy $R2 "$R2-b"
  ${EndIf}
  ClearErrors
  Rename "$R9" "$R2"
  ${If} ${Errors}
    DetailPrint "[!] 改名失败（$R2 也不可用），无法替换 $R9"
    StrCpy $R5 1
    Goto verify
  ${EndIf}
  DetailPrint "  旧文件已改名为 $R2"
  ClearErrors
  CopyFiles /SILENT "$R8" "$R9"
  ${If} ${Errors}
    DetailPrint "[!] 旧文件已挪走，但新文件写不进 $R9"
    StrCpy $R5 1
  ${EndIf}

  verify:
  ; 大小比对是这里唯一的硬校验：只查 FileExists 会把「旧文件还在」当成成功。
  ; FileFunc 的 GetSize 只统计目录（内部 IfFileExists "$path\*.*"），这里用
  ; FileSeek 到 END 直接拿长度——没有外部依赖，也不涉及结构体封送。
  ClearErrors
  FileOpen $R3 "$R8" r
  ${If} ${Errors}
    StrCpy $R6 -1
  ${Else}
    FileSeek $R3 0 END $R6
    FileClose $R3
  ${EndIf}
  ClearErrors
  FileOpen $R3 "$R9" r
  ${If} ${Errors}
    StrCpy $R4 -1
  ${Else}
    FileSeek $R3 0 END $R4
    FileClose $R3
  ${EndIf}
  ${If} $R6 != $R4
    DetailPrint "[!] $R9 大小不符（应为 $R6，实际 $R4）"
    StrCpy $R5 1
  ${EndIf}
  ; 顺手清历史残留；删不掉（映射中）就算了，不影响本次安装
  Delete "$R9.old"
  Delete "$R9.old-*"
FunctionEnd

; 调 PowerShell 脚本并检查退出码。入参：$R7 = 完整命令行
Function RunPs
  nsExec::ExecToLog '$R7'
  Pop $R6
  ${If} $R6 != "0"
    DetailPrint "[!] 脚本返回 $R6"
    MessageBox MB_ICONSTOP|MB_OK "注册步骤失败（退出码 $R6）。$\r$\n$\r$\n详细信息见安装日志。"
    SetErrorLevel 1
    Abort
  ${EndIf}
FunctionEnd

; ── 安装 ────────────────────────────────────────────────────────
Section "主程序" SecMain
  SetOutPath "$INSTDIR"

  ; 安装前先确认 DLL 真的存在，避免打出一个空壳安装包
  !if /FILEEXISTS "${DLL64}"
  !else
    !error "找不到 ${DLL64}，请先运行 build.ps1 -NoUserDict"
  !endif
  !if /FILEEXISTS "${DLL32}"
  !else
    !error "找不到 ${DLL32}，请先运行 build.ps1 -NoUserDict"
  !endif

  ; 必须用 File 把 DLL 内嵌进安装包（CopyFiles 是运行时拷贝，用户机上没有
  ; 构建目录）。先解到 $PLUGINSDIR 暂存，再拷到 $INSTDIR —— 这样「目标被占用
  ; 就改名 .old 再拷」的兜底逻辑才有意义。$PLUGINSDIR 退出时自动清空。
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\ime"
  File /oname=feihe.dll   "${DLL64}"
  File /oname=feihe32.dll "${DLL32}"

  SetOutPath "$INSTDIR"
  StrCpy $R5 0

  DetailPrint "部署 64 位 DLL…"
  StrCpy $R8 "$PLUGINSDIR\ime\feihe.dll"
  StrCpy $R9 "$INSTDIR\feihe.dll"
  Call CopyOrReplace

  DetailPrint "部署 32 位 DLL…"
  StrCpy $R8 "$PLUGINSDIR\ime\feihe32.dll"
  StrCpy $R9 "$INSTDIR\feihe32.dll"
  Call CopyOrReplace

  ; 兜底校验：CopyOrReplace 的任何失败路径都不许静默——DLL 没落地还继续注册的话，
  ; 症状是「装完了但输入法不存在」，比当场报错难查得多。失败姿势和 RunPs 一致。
  ${If} $R5 == "0"
  ${AndIf} ${FileExists} "$INSTDIR\feihe.dll"
  ${AndIf} ${FileExists} "$INSTDIR\feihe32.dll"
  ${Else}
    MessageBox MB_ICONSTOP|MB_OK "DLL 部署失败：$INSTDIR\feihe.dll 未正确写入。$\r$\n$\r$\n多半是旧文件还被某个程序（ctfmon、打开中的记事本或浏览器）占用着。$\r$\n请关掉这些程序后重新运行安装程序；也可以先把 $INSTDIR 里 feihe.dll.old* 之类改个名字再装。"
    SetErrorLevel 1
    Abort
  ${EndIf}

  ; 注册脚本一起装上：卸载要用，出问题时也能手工重跑
  File "install_tip.ps1"
  File "uninstall_tip.ps1"

  ; 用户手册。文件名带中文，方便用户直接双击打开；内容随版本更新，不写注册表。
  DetailPrint "部署用户手册…"
  File "/oname=使用帮助.md" "${HELPDOC}"

  ; 卸载程序
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; ── 注册（COM + CTF TIP 双视图 + 用户键盘列表）──
  ; 自检模式跳过：它没有管理员权限，也不该改动本机真实输入法列表。
  ; 自检只验证「两个 DLL 有没有被完整、正确地内嵌并落到 $INSTDIR」。
  !ifndef SELFTEST
  DetailPrint "注册 COM / TSF 组件…"
  StrCpy $R7 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\install_tip.ps1" -DllPath "$INSTDIR\feihe.dll" -DllPath32 "$INSTDIR\feihe32.dll"'
  Call RunPs

  ; ── 清理老的 ProgramData 副本（register.cmd 时代留下的）──
  ReadEnvStr $R0 "ProgramData"
  StrCpy $R1 "$R0\Feihe"
  ${If} $R1 != ""
  ${AndIf} $R1 != "$INSTDIR"
  ${AndIf} ${FileExists} "$R1\feihe.dll"
    ; 不用 /REBOOTOK：那会置 NSIS 的重启标记，完成页就多出「重启计算机」。
    ; 删不掉说明旧 DLL 还被某个进程映射着——留着无害，注册项已由 install_tip.ps1
    ; 清理，不会被再加载；重启后用户自己删掉即可，不该因此拦住本次安装。
    DetailPrint "清理旧位置 $R1（删不掉的留着，不安排重启删除）"
    Delete "$R1\feihe.dll"
    Delete "$R1\feihe32.dll"
    ${If} ${FileExists} "$R1\feihe.dll"
      DetailPrint "  旧 DLL 仍被占用，保留在 $R1，可稍后手动删除"
    ${EndIf}
    ; register.cmd 时代留下的改名残留形如 feihe.dll.old-141324，通配要宽一点
    Delete "$R1\*.old"
    Delete "$R1\*.old-*"
    Delete "$R1\install_tip.ps1"
    Delete "$R1\uninstall_tip.ps1"
    RMDir "$R1"
  ${EndIf}

  ; ── 控制面板「程序和功能」条目 ──
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayVersion"  "${APP_VER}"
  WriteRegStr   HKCU "${UNINST_KEY}" "Publisher"       "${APP_PUBLISHER}"
  WriteRegStr   HKCU "${UNINST_KEY}" "URLInfoAbout"    "${APP_URL}"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayIcon"     "$INSTDIR\feihe.dll,-101"
  WriteRegStr   HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr   HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  ; GetSize 以 KB 返回，$0 = 低 32 位、$1 = 高 32 位。DLL 只有几 MB，$0 够用。
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
  !endif
SectionEnd

; ── 卸载 ────────────────────────────────────────────────────────
Section "Uninstall"
  ; 先注销，再删文件。顺序反了的话 CLSID 会指向一个不存在的路径，
  ; 切到飞鹤时宿主直接加载失败，而且没地方报错。
  ; uninstall_tip.ps1 末尾会重启 ctfmon：映射一释放，下面的 Delete 才真正删得掉
  ; 被占用的 DLL（否则只能留下一堆「重启后可删」的残留，RMDir 也会失败）。
  ${If} ${FileExists} "$INSTDIR\uninstall_tip.ps1"
    DetailPrint "注销 COM / TSF 组件…"
    StrCpy $R7 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\uninstall_tip.ps1"'
    nsExec::ExecToLog '$R7'
    Pop $R6
    ${If} $R6 != "0"
      DetailPrint "[!] 注销脚本返回 $R6，继续删除文件"
    ${EndIf}
  ${EndIf}

  ; 同样不用 /REBOOTOK。uninstall_tip.ps1 末尾已经杀过 ctfmon，映射一释放这里
  ; 当场就能删；万一 32 位宿主还占着，就留着文件并说清楚——绝不能为删一个文件
  ; 弹「要重启吗」，卸载时多这一步最招人烦。
  Delete "$INSTDIR\feihe.dll"
  Delete "$INSTDIR\feihe32.dll"
  ${If} ${FileExists} "$INSTDIR\feihe.dll"
  ${OrIf} ${FileExists} "$INSTDIR\feihe32.dll"
    DetailPrint "[!] DLL 仍被占用，注册项已移除；重启后可手动删除 $INSTDIR"
  ${EndIf}
  Delete "$INSTDIR\install_tip.ps1"
  Delete "$INSTDIR\uninstall_tip.ps1"
  Delete "$INSTDIR\使用帮助.md"
  Delete "$INSTDIR\*.old"
  Delete "$INSTDIR\uninstall.exe"

  ; 用户数据**故意保留**：config.ini / freq.txt / phrases.txt / pinned.txt 是用户
  ; 自己的打字习惯，升级、重装都还要用；留在原处也让这个文件夹继续是一份完整备份。
  ; 所以这里是 RMDir（不带 /r）——目录非空就删不掉，正是预期行为。
  ${If} ${FileExists} "$INSTDIR\config.ini"
    DetailPrint "保留用户数据：$INSTDIR（config.ini / freq.txt / phrases.txt / pinned.txt）"
  ${EndIf}
  RMDir "$INSTDIR"

  ; 只删自己的（HKCU）。旧版写在 HKLM 的条目删不掉（没权限），
  ; 提权失败也不该让卸载半途而废 —— 用户可以在「程序和功能」里手动清掉那条僵尸项。
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
