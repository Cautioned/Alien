# Set paths
$CURRENT_DIR = $PWD
$DLL_PATH = "$CURRENT_DIR/lib/64/libmpv-2.dll"

# Create def file from our DLL
& "C:/msys64/mingw64/bin/gendef.exe" "$DLL_PATH"

# Generate both lib files from def
& "C:/msys64/mingw64/bin/dlltool.exe" -d libmpv-2.def -l mpv.lib -D libmpv-2.dll
& "C:/msys64/mingw64/bin/dlltool.exe" -d libmpv-2.def -l libmpv-2.lib -D libmpv-2.dll

# Clean up def file
Remove-Item libmpv-2.def 