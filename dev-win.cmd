@echo off
set PKG_CONFIG=C:\Program Files\gstreamer\1.0\msvc_x86_64\bin\pkg-config.exe
set PKG_CONFIG_PATH=C:\Program Files\gstreamer\1.0\msvc_x86_64\lib\pkgconfig;C:\tools\ffmpeg-n7.1-latest-win64-gpl-shared-7.1\lib\pkgconfig
set PKG_CONFIG_ALLOW_CROSS=1
set FFMPEG_DIR=C:\tools\ffmpeg-n7.1-latest-win64-gpl-shared-7.1
set PATH=C:\Program Files\gstreamer\1.0\msvc_x86_64\bin;C:\tools\ffmpeg-n7.1-latest-win64-gpl-shared-7.1\bin;%PATH%
tauri dev --target x86_64-pc-windows-msvc
