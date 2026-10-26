; NSIS installer hooks (tauri-bundler 2.10 template, `bundle.windows.nsis.installerHooks`).
;
; Migration from the old product name "MineUP" (<= 1.0.6) to "MineDed".
; Tauri derives the uninstall key, the `Software\<publisher>\<product>` key and the default
; install dir from PRODUCTNAME, so without this hook the new installer would install next to
; the old one. The bundle identifier is unchanged, so app data (%APPDATA%\ru.dedseckms.mineup)
; and keyring entries are shared and must survive.
;
; The old uninstaller is run silently (`/S`) and without `/UPDATE`: it then removes its own
; shortcuts and Run entry, and it never deletes app data in silent mode (that needs the
; "delete app data" checkbox, which is only shown on the interactive confirm page).

!define OLD_PRODUCTNAME "MineUP"
!define OLD_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${OLD_PRODUCTNAME}"
!define OLD_MANUPRODUCTKEY "Software\${MANUFACTURER}\${OLD_PRODUCTNAME}"

Var MigratedOldProduct

; Uninstall the old product registered under `ROOT` (HKCU or HKLM), keeping app data.
!macro _MigrateOldProduct ROOT
  ReadRegStr $R8 ${ROOT} "${OLD_UNINSTKEY}" "UninstallString"
  ${If} $R8 != ""
    ; Install dir: `Software\<publisher>\MineUP` (unquoted), else InstallLocation (quoted).
    ReadRegStr $R9 ${ROOT} "${OLD_MANUPRODUCTKEY}" ""
    ${If} $R9 == ""
      ReadRegStr $R9 ${ROOT} "${OLD_UNINSTKEY}" "InstallLocation"
      ${StrLoc} $R7 $R9 '"' ">"
      ${If} $R7 == "0"
        StrCpy $R9 $R9 -1 1
      ${EndIf}
    ${EndIf}
    DetailPrint "Removing previous ${OLD_PRODUCTNAME} installation: $R9"
    ClearErrors
    ; `_?=` runs the uninstaller in place so ExecWait really waits for it.
    ${If} $R9 != ""
      ExecWait '$R8 /S _?=$R9' $R7
    ${Else}
      ExecWait '$R8 /S' $R7
    ${EndIf}
    ${If} ${Errors}
    ${OrIf} $R7 <> 0
      DetailPrint "${OLD_PRODUCTNAME} uninstaller failed ($R7), continuing"
    ${Else}
      StrCpy $MigratedOldProduct 1
      ; An in-place uninstaller cannot delete itself or its directory.
      ${If} $R9 != ""
        Delete "$R9\uninstall.exe"
        RMDir "$R9"
      ${EndIf}
      ; Leftovers the old uninstaller keeps in silent mode (install dir, installer language).
      DeleteRegKey ${ROOT} "${OLD_MANUPRODUCTKEY}"
      DeleteRegKey /ifempty ${ROOT} "Software\${MANUFACTURER}"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  StrCpy $MigratedOldProduct 0
  !insertmacro _MigrateOldProduct HKCU
  !insertmacro _MigrateOldProduct HKLM

  ; Old shortcuts, in case the uninstaller did not remove them (e.g. retargeted by the user).
  Delete "$SMPROGRAMS\${OLD_PRODUCTNAME}.lnk"
  Delete "$SMPROGRAMS\${OLD_PRODUCTNAME}\${OLD_PRODUCTNAME}.lnk"
  RMDir "$SMPROGRAMS\${OLD_PRODUCTNAME}"
  Delete "$DESKTOP\${OLD_PRODUCTNAME}.lnk"

  ; The updater passes /UPDATE, which makes the template skip shortcut creation. After a
  ; migration the old shortcuts are gone, so treat this run as a fresh install from here on
  ; (only affects shortcut creation; the reinstall page has already run).
  ${If} $MigratedOldProduct = 1
    StrCpy $UpdateMode 0
  ${EndIf}
!macroend
