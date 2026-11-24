; Custom steps for the wallpaper Lite NSIS installer and uninstaller.
; The wallpaper host runs apart from the app window, so it has to be stopped
; before files are replaced or removed.

!macro WALLPAPER_STOP_PROCESSES
  ; Stop the app first so it cannot start the host again, then the host.
  nsExec::Exec 'taskkill /F /IM wallpaper-ui.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM wallpaper-host.exe'
  Pop $0
  Sleep 500
!macroend

Var WallpaperHadOldAutostart

; Builds made before the product was renamed to "wallpaper Lite" were installed
; as "wallpaper-lite". The installer treats that as another product, so it would
; install a second copy next to it, and both copies share the host's pipe, mutex
; and settings. Remove the old copy first (silently, keeping the user's data) and
; carry its launch-at-login choice over to the new install.
!macro WALLPAPER_REMOVE_OLD_NAME_INSTALL
  StrCpy $WallpaperHadOldAutostart "0"
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\wallpaper-lite" "UninstallString"
  ${If} $0 != ""
    ReadRegStr $1 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "wallpaper-lite"
    ${If} $1 != ""
      StrCpy $WallpaperHadOldAutostart "1"
    ${EndIf}
    ; The uninstaller copies itself elsewhere and returns at once, so wait for it
    ; to remove its own registry entry (up to about 30 seconds).
    Exec '$0 /S'
    StrCpy $2 0
    wallpaper_wait_old_uninstall:
      Sleep 500
      IntOp $2 $2 + 1
      ReadRegStr $3 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\wallpaper-lite" "UninstallString"
      ${If} $3 != ""
      ${AndIf} $2 < 60
        Goto wallpaper_wait_old_uninstall
      ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro WALLPAPER_STOP_PROCESSES
  !insertmacro WALLPAPER_REMOVE_OLD_NAME_INSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $WallpaperHadOldAutostart == "1"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "wallpaper-lite" '"$INSTDIR\wallpaper-ui.exe" --autostart'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro WALLPAPER_STOP_PROCESSES
  ; Launch at login belongs to the app, so it goes with the app. Windows keeps
  ; the Task Manager on/off choice separately.
  ; An upgrade runs the old uninstaller too and must keep launch at login.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "wallpaper-lite"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "wallpaper-lite"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Library, editor settings and display profiles live in %APPDATA%\WallpaperLite,
  ; which the stock "delete app data" option does not cover. Only remove it when
  ; the user ticked that option. Source images and videos are never touched.
  ; An upgrade runs the old uninstaller too; it must never wipe user data.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    RMDir /r "$APPDATA\WallpaperLite"
  ${EndIf}
!macroend
