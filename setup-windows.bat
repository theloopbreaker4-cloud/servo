@echo off
echo Setting up Servo for Windows...

:: Find libEGL.dll and libGLESv2.dll from Chrome
set CHROME_DIR=C:\Program Files\Google\Chrome\Application
set TARGET_DIR=%~dp0target\debug

:: Find latest Chrome version
for /f %%i in ('dir "%CHROME_DIR%" /b /ad /o-n') do (
    set CHROME_VER=%%i
    goto :found
)

:found
echo Found Chrome: %CHROME_VER%

if not exist "%TARGET_DIR%" mkdir "%TARGET_DIR%"

copy "%CHROME_DIR%\%CHROME_VER%\libEGL.dll" "%TARGET_DIR%\" /y
copy "%CHROME_DIR%\%CHROME_VER%\libGLESv2.dll" "%TARGET_DIR%\" /y

echo Done! DLLs copied to %TARGET_DIR%
pause
