; repair.nsi — 飞鹤拼音「修复安装状态」小工具
;
; 用途：安装包跑完但注册步骤失败、用户手动删过注册表、或者换过安装目录之后，
; 双击这个 exe 就能把 COM / CTF TIP / 用户键盘列表重新写一遍，**不用重装**。
;
; 设计取舍
; --------
; 1. **一个文件都不动。** 它只重新注册，不覆盖 DLL、不碰 config.ini / freq.txt /
;    phrases.txt / pinned.txt。用户数据是用户的，修复不该有副作用——这也让它能
;    在「DLL 正被浏览器映射着」的时候照样跑。
; 2. install_tip.ps1 **内嵌在 exe 里**，不用安装目录那份：那份可能被删掉、被改过、
;    或者本身就是旧版本（旧版本有个把 TIP 定义删掉的 bug）。
; 3. 安装目录优先读卸载注册项，读不到就用 exe 自身所在目录——用户把 exe 拷进
;    安装目录双击是最常见的用法。
; 4. **必须管理员**（改为 admin）。注册要写 HKLM——写 HKCU 的话系统读不到，
;    表现就是「设置里多出一个不可用的输入法、Win+Space 里没有飞鹤」。修复工具
;    如果还按用户级跑，恰好在修不了的那个故障上无能为力。代价是弹一次 UAC。
; 5. 结果必须**看得见**：成功弹一个、失败弹一个并带上退出码。静默结束等于让用户
;    自己猜，而这正是「装完没有输入法」这类问题的根源。
; 6. **先体检再动手。** 双击它的人多数只是「输入法没反应，想找个东西点一下」，
;    注册往往是好的。跑 install_tip.ps1 -CheckOnly（同一份脚本、同一套路径常量），
;    全对就弹「无需修复」直接退出，一个字都不写；有缺项才真的修，并把缺了哪些
;    打进日志。这样它既是修复工具，也是一个能自证「现在到底正不正常」的体检工具
;    ——这是安装包替代不了的那部分价值。
;
; 编译：makensis /DOUTDIR=<dist> repair.nsi

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APP_NAME "飞鹤拼音"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FeiheIME"

!ifndef OUTDIR
  !define OUTDIR "..\dist"
!endif

Name "${APP_NAME} - 修复安装状态"
OutFile "${OUTDIR}\Feihe-Repair.exe"
RequestExecutionLevel admin
ShowInstDetails show
BrandingText "${APP_NAME} 修复工具"

Icon "assets\feihe.ico"

VIProductVersion "1.0.0.0"
VIAddVersionKey /LANG=2052 "ProductName"     "${APP_NAME} 修复工具"
VIAddVersionKey /LANG=2052 "FileDescription" "重新注册输入法（不改动任何文件）"
VIAddVersionKey /LANG=2052 "FileVersion"     "1.0.0.0"
VIAddVersionKey /LANG=2052 "LegalCopyright"  "MIT License"

!define MUI_ICON "assets\feihe.ico"
!define MUI_FINISHPAGE_NOAUTOCLOSE
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

; 没有「完成页」：修复是秒级动作，做完直接弹结论框，少一次点击。
; SilentInstall normal 让进度窗口正常显示（用户能看到 [ok] / [FAIL] 那几行）。
SilentInstall normal

Section "修复"
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=install_tip.ps1 "install_tip.ps1"

  ; ── 定位安装目录 ──
  ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
  ${If} $0 == ""
    StrCpy $0 "$EXEDIR"
    DetailPrint "注册表里没有安装路径，用本程序所在目录：$0"
  ${Else}
    DetailPrint "安装目录（来自注册表）：$0"
  ${EndIf}

  ${IfNot} ${FileExists} "$0\feihe.dll"
    MessageBox MB_ICONSTOP|MB_OK "在 $0 里找不到 feihe.dll。$\r$\n$\r$\n请先运行安装程序，或者把这个修复工具放到安装目录里再双击。"
    Abort
  ${EndIf}

  ; ── 拼命令行 ──
  StrCpy $1 'powershell -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\install_tip.ps1" -DllPath "$0\feihe.dll"'
  ${If} ${FileExists} "$0\feihe32.dll"
    StrCpy $1 '$1 -DllPath32 "$0\feihe32.dll"'
  ${Else}
    DetailPrint "没找到 feihe32.dll：只注册 64 位宿主"
  ${EndIf}

  ; ── 先体检：本来就是好的就什么都不做 ──
  ; 大多数双击它的场合注册是正常的（用户只是「输入法没反应，想找个东西点一下」）。
  ; 白写一遍注册不会坏，但会让人以为真出过问题，也浪费一次 UAC 之外的等待。
  ; 体检用的是同一份脚本、同一套路径常量，不会出现「NSIS 说好、脚本说坏」。
  StrCpy $2 '$1 -CheckOnly'
  DetailPrint "检查当前注册状态…"
  nsExec::ExecToLog '$2'
  Pop $3
  ${If} $3 == "0"
    MessageBox MB_ICONINFORMATION|MB_OK "当前注册状态正常，不需要修复。$\r$\n$\r$\n如果你在 Win+Space 里还是看不到「${APP_NAME}」，多半是 shell 的输入法列表缓存没刷新——注销后重新登录一次即可（比重启快）。"
    Goto done
  ${EndIf}
  DetailPrint "[fix] 需要修复（上面 [need] 行是缺的项），开始重写注册…"

  DetailPrint "重新注册 COM / TSF 组件…"
  DetailPrint "  $1"
  nsExec::ExecToLog '$1'
  Pop $2

  ${If} $2 == "0"
    MessageBox MB_ICONINFORMATION|MB_OK "修复完成。$\r$\n$\r$\n用 Win+Space 或 Ctrl+Shift 切到「${APP_NAME}」。$\r$\n如果列表里还看不到它，注销后重新登录一次（列表由 shell 缓存）。"
  ${Else}
    MessageBox MB_ICONSTOP|MB_OK "修复失败（脚本退出码 $2）。$\r$\n$\r$\n本窗口的日志里有具体原因，最后几行就是。$\r$\n常见原因：安装目录里的 feihe.dll 被安全软件锁住，或者没有以管理员身份运行（注册要写 HKLM）。"
    SetErrorLevel 1
  ${EndIf}

  done:
SectionEnd
