@echo off
setlocal

:: Set MPV source directory
set MPV_SOURCE=src-tauri

:: Check if MPV DLL exists
if not exist "%MPV_SOURCE%\libmpv-2.dll" (
    echo ERROR: MPV DLL not found at %MPV_SOURCE%\libmpv-2.dll
    echo Please ensure libmpv-2.dll exists in the src-tauri directory
    exit /b 1
)

:: Copy MPV DLL to release directory
echo Copying MPV DLL to release directory...
copy /Y "%MPV_SOURCE%\libmpv-2.dll" "src-tauri\target\release\libmpv-2.dll"

:: Generate the lib file from the DLL (if not already generated)
if not exist "%MPV_SOURCE%\libmpv-2.lib" (
    echo Generating MPV lib file...
    powershell -ExecutionPolicy Bypass -File "%MPV_SOURCE%\generate-lib.ps1"
)

:: Run the build
echo Building application...
npm run dev

if errorlevel 1 (
    echo Build failed
    exit /b 1
)

echo Build completed successfully 