@echo off
setlocal

:: Set MPV source directory
set MPV_SOURCE=src-tauri

:: --- Securely Prompt for Signing Credentials --- 
echo.
echo Please provide the signing key details:
echo.

set /p TAURI_PRIVATE_KEY="Enter FULL path to your TAURI_PRIVATE_KEY file: "

:: Check if the path was entered
if not defined TAURI_PRIVATE_KEY (
    echo ERROR: Private key path cannot be empty.
    exit /b 1
)

:: Check if the file exists (basic check)
if not exist "%TAURI_PRIVATE_KEY%" (
    echo WARNING: Private key file not found at the specified path: %TAURI_PRIVATE_KEY%
    echo          Continuing, but the build might fail if the path is incorrect.
)

echo.
set /p TAURI_KEY_PASSWORD="Enter password for the key (leave blank if none): "

:: --- End Prompt Section ---


:: Check if MPV DLL exists
if not exist "%MPV_SOURCE%\libmpv-2.dll" (
    echo ERROR: MPV DLL not found at %MPV_SOURCE%\libmpv-2.dll
    echo Please ensure libmpv-2.dll exists in the src-tauri directory
    exit /b 1
)

:: Copy MPV DLL to release directory
echo Copying MPV DLL to release directory...
copy /Y "%MPV_SOURCE%\libmpv-2.dll" "src-tauri\target\release\libmpv-2.dll"

:: Check and generate the lib file from the DLL
if not exist "%MPV_SOURCE%\libmpv-2.lib" (
    echo Generating MPV lib file...
    powershell -ExecutionPolicy Bypass -File "%MPV_SOURCE%\generate-lib.ps1"

)

:: Run the build (Uses the environment variables set via prompt)
echo Building application...
npm run build --release

if errorlevel 1 (
    echo Build failed
    exit /b 1
)

echo Build completed successfully 