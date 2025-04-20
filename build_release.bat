@echo off
setlocal

:: Set MPV source directory
set MPV_SOURCE=src-tauri

:: --- Securely Prompt for Signing Credentials --- 
echo.
echo Please provide the signing key details:
echo.

set /p KEY_PATH_INPUT="Enter FULL path to your private key file (no quotes): "

:: Check if the path was entered
if not defined KEY_PATH_INPUT (
    echo ERROR: Private key path cannot be empty.
    exit /b 1
)

:: Set both v2 and v1 style environment variables from the input
set TAURI_PRIVATE_KEY=%KEY_PATH_INPUT%
set TAURI_SIGNING_PRIVATE_KEY=%KEY_PATH_INPUT%


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

:: Generate the lib file from the DLL
echo Generating MPV lib file...
powershell -ExecutionPolicy Bypass -File "%MPV_SOURCE%\generate-lib.ps1"

:: Run the build (Uses the environment variables set via prompt)
echo Building application for release...
npm run tauri build --release

if errorlevel 1 (
    echo Build failed
    exit /b 1
)

echo Build completed successfully 