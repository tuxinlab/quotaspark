; 安装器自定义钩子（tauri.conf.json → bundle.windows.nsis.installerHooks）
;
; POSTINSTALL：刷新系统图标缓存。
; 覆盖安装会替换 exe 内嵌图标（0.1.9 起由蓝紫火焰换成琥珀火焰），但 Explorer
; 按「文件路径」缓存任务栏 / 快捷方式图标且不主动失效——用户从旧版本升级后，
; 任务栏会残留旧图标，与运行中的新图标并存。ie4uinit -show 让缓存失效，
; 任务栏与快捷方式立即重读新图标，无需重启资源管理器。
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$SysDir\ie4uinit.exe" -show'
!macroend
