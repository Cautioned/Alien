@echo off
set MPV_SOURCE=%~dp0lib
echo Setting MPV_SOURCE to: %MPV_SOURCE%

REM Build the project
call cargo tauri build

REM Copy MPV DLL to the release directory, please obtain the DLL yourself from MPV or use the one in our release
echo Copying MPV DLL to release directory...
xcopy /Y "%~dp0lib\64\libmpv-2.dll" "%~dp0target\release\"

cd ..
npm run tauri build 