; feihe.nsi — 飞鹤拼音 安装程序（NSIS 3.x / Unicode）
;
; 设计取舍
; --------
; 1. **注册逻辑不在这里重写**。COM / CTF TIP / 用户键盘列表这三件事全部交给
;    install_tip.ps1、uninstall_tip.ps1 —— 它们是唯一真相，开发时 register.cmd
;    用的也是同一份。安装包只负责「提权 + 拷文件 + 调脚本 + 卸载时反向调」。
;    否则 Pascal 里再来一套，两份实现必然漂移。
; 2. 装到 Program Files（不是老的 C:\ProgramData\Feihe）。安装包该有安装包的样子，
;    老的 ProgramData 副本会在安装时尽力清理（锁住的排到重启删）。
; 3. 覆盖 DLL 时先 Delete 再 Rename 兜底：ctfmon / explorer 常常已经把旧 DLL
;    映射进进程，直接 Copy 会失败。Windows 允许**重命名**已映射的文件，
;    所以改名成 .old 再拷新文件就行，.old 排到重启删。
;
; 编译：makensis /DARCH=x64 feihe.nsi   （或直接跑 build_setup.ps1）

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; ── 基本信息 ────────────────────────────────────────────────────
!define APP_NAME      "飞鹤拼音"
!define APP_NAME_EN   "Feihe IME"
!define APP_VER       "0.1.0"
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

; ── 编译器属性 ──────────────────────────────────────────────────
Name "${APP_NAME}"
OutFile "${OUTDIR}\Feihe-Setup-${APP_VER}.exe"
InstallDir "$PROGRAMFILES64\Feihe"
InstallDirRegKey HKLM "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
SetCompressorDictSize 32
ShowInstDetails show
ShowUninstDetails show
BrandingText "${APP_NAME} ${APP_VER} — 小鹤双拼 / 全拼 输入法"

VIProductVersion "0.1.0.0"
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
!define MUI_FINISHPAGE_TEXT "${APP_NAME} 已安装。$\r$\n$\r$\n用 Win+Space 或 Ctrl+Shift 切换到「${APP_NAME}」。$\r$\n$\r$\n如果输入法列表里还看不到它，请注销后重新登录（列表由 shell 缓存）。"
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

; 覆盖一个可能被占用（已被 ctfmon / explorer 映射）的文件。
; 入参：$R8 = 源文件，$R9 = 目标文件
Function CopyOrReplace
  ClearErrors
  CopyFiles /SILENT "$R8" "$R9"
  IfErrors 0 done
    DetailPrint "  $R9 被占用，改名旧文件后重试"
    ; Windows 允许重命名已映射的文件，这是绕开「无法覆盖正在使用的 DLL」的正路
    Delete "$R9.old"
    ClearErrors
    Rename "$R9" "$R9.old"
    ClearErrors
    CopyFiles /SILENT "$R8" "$R9"
    IfErrors 0 done
      DetailPrint "  [!] 仍然失败，排到重启后替换"
      Delete /REBOOTOK "$R9"
      CopyFiles /SILENT "$R8" "$R9"
    Goto done
  done:
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
  DetailPrint "部署 64 位 DLL…"
  StrCpy $R8 "$PLUGINSDIR\ime\feihe.dll"
  StrCpy $R9 "$INSTDIR\feihe.dll"
  Call CopyOrReplace

  DetailPrint "部署 32 位 DLL…"
  StrCpy $R8 "$PLUGINSDIR\ime\feihe32.dll"
  StrCpy $R9 "$INSTDIR\feihe32.dll"
  Call CopyOrReplace

  ; 注册脚本一起装上：卸载要用，出问题时也能手工重跑
  File "install_tip.ps1"
  File "uninstall_tip.ps1"

  ; 卸载程序
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; ── 注册（COM + CTF TIP 双视图 + 用户键盘列表）──
  DetailPrint "注册 COM / TSF 组件…"
  StrCpy $R7 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\install_tip.ps1" -DllPath "$INSTDIR\feihe.dll" -DllPath32 "$INSTDIR\feihe32.dll"'
  Call RunPs

  ; ── 清理老的 ProgramData 副本（register.cmd 时代留下的）──
  ReadEnvStr $R0 "ProgramData"
  StrCpy $R1 "$R0\Feihe"
  ${If} $R1 != ""
  ${AndIf} $R1 != "$INSTDIR"
  ${AndIf} ${FileExists} "$R1\feihe.dll"
    DetailPrint "清理旧位置 $R1（正在使用的排到重启后删）"
    Delete /REBOOTOK "$R1\feihe.dll"
    Delete /REBOOTOK "$R1\feihe32.dll"
    ; register.cmd 时代留下的改名残留形如 feihe.dll.old-141324，通配要宽一点
    Delete "$R1\*.old"
    Delete "$R1\*.old-*"
    Delete "$R1\install_tip.ps1"
    Delete "$R1\uninstall_tip.ps1"
    RMDir "$R1"
  ${EndIf}

  ; ── 控制面板「程序和功能」条目 ──
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayVersion"  "${APP_VER}"
  WriteRegStr   HKLM "${UNINST_KEY}" "Publisher"       "${APP_PUBLISHER}"
  WriteRegStr   HKLM "${UNINST_KEY}" "URLInfoAbout"    "${APP_URL}"
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayIcon"     "$INSTDIR\feihe.dll,-101"
  WriteRegStr   HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr   HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  ; GetSize 以 KB 返回，$0 = 低 32 位、$1 = 高 32 位。DLL 只有几 MB，$0 够用。
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

; ── 卸载 ────────────────────────────────────────────────────────
Section "Uninstall"
  ; 先注销，再删文件。顺序反了的话 CLSID 会指向一个不存在的路径，
  ; 切到飞鹤时宿主直接加载失败，而且没地方报错。
  ${If} ${FileExists} "$INSTDIR\uninstall_tip.ps1"
    DetailPrint "注销 COM / TSF 组件…"
    StrCpy $R7 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\uninstall_tip.ps1"'
    nsExec::ExecToLog '$R7'
    Pop $R6
    ${If} $R6 != "0"
      DetailPrint "[!] 注销脚本返回 $R6，继续删除文件"
    ${EndIf}
  ${EndIf}

  Delete "$INSTDIR\feihe.dll"
  Delete "$INSTDIR\feihe32.dll"
  Delete "$INSTDIR\install_tip.ps1"
  Delete "$INSTDIR\uninstall_tip.ps1"
  Delete "$INSTDIR\*.old"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd
