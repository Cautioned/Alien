@echo off
setlocal

:: Set MPV source directory
set MPV_SOURCE=src-tauri\lib\64

:: Check if MPV DLL exists
if not exist "%MPV_SOURCE%\libmpv-2.dll" (
    echo ERROR: MPV DLL not found at %MPV_SOURCE%\libmpv-2.dll
    echo Please ensure libmpv-2.dll exists in the lib/64 directory
    exit /b 1
)

:: Copy MPV DLL to release directory
echo Copying MPV DLL to release directory...
copy /Y "%MPV_SOURCE%\libmpv-2.dll" "src-tauri\target\release\libmpv-2.dll"

:: Run the build
echo Building application...
npm run build

if errorlevel 1 (
    echo Build failed
    exit /b 1
)

echo Build completed successfully 