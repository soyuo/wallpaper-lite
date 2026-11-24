; Custom steps for the wallpaper-lite NSIS installer and uninstaller.
; The wallpaper host runs apart from the app window, so it has to be stopped
; before files are replaced or removed.

!macro WALLPAPER_STOP_PROCESSES
  nsExec::Exec 'taskkill /F /IM wallpaper-host.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM wallpaper-ui.exe'
  Pop $0
  Sleep 500
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro WALLPAPER_STOP_PROCESSES
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro WALLPAPER_STOP_PROCESSES
  ; Launch at login belongs to the app, so it goes with the app. Windows keeps
  ; the Task Manager on/off choice separately.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "wallpaper-lite"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "wallpaper-lite"
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
