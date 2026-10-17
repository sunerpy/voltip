; Hooks into Tauri's NSIS installer (bundle.windows.nsis.installerHooks, tauri.windows.conf.json).
;
; In-app updates (docs/dictation.md §9, user request 2026-10-17): the updater starts this installer
; with `/UPDATE`. Releases after 0.1.6 ask for the silent mode (`/S`), which opens no window at all.
; 0.1.6 and earlier ask for the passive mode (`/P`), whose progress window was the installer box
; users saw; for such a run the window is made invisible before its first frame: layered and fully
; transparent, kept out of the taskbar and never activated, then hidden. A later ShowWindow by the
; pages still paints nothing. An install the user starts keeps its pages. The same approach as
; winer's updater (sunerpy/winer, app/src-tauri/nsis/installer-hooks.nsh).
!define MUI_CUSTOMFUNCTION_GUIINIT VoltipHideUpdateWindow

Function VoltipHideUpdateWindow
  Push $0
  Push $1
  Push $R0
  ClearErrors
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    ClearErrors
    ${GetOptions} $CMDLINE "/UPDATE" $R0
    ${IfNot} ${Errors}
      ; GWL_EXSTYLE (-20) plus WS_EX_LAYERED (0x80000), WS_EX_NOACTIVATE (0x8000000) and
      ; WS_EX_TOOLWINDOW (0x80).
      System::Call 'user32::GetWindowLongW(p $HWNDPARENT, i -20) i .r0'
      IntOp $0 $0 | 0x08080080
      System::Call 'user32::SetWindowLongW(p $HWNDPARENT, i -20, i r0) i .r1'
      ; LWA_ALPHA (2) with an alpha of 0.
      System::Call 'user32::SetLayeredWindowAttributes(p $HWNDPARENT, i 0, i 0, i 2) i .r1'
      HideWindow
    ${EndIf}
  ${EndIf}
  Pop $R0
  Pop $1
  Pop $0
FunctionEnd

; A second guard right before the files are copied, for a passive update run.
!macro NSIS_HOOK_PREINSTALL
  ${If} $UpdateMode = 1
  ${AndIf} $PassiveMode = 1
    HideWindow
  ${EndIf}
!macroend
