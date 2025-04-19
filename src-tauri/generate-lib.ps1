# Set paths
$SCRIPT_DIR = Split-Path -Parent $MyInvocation.MyCommand.Path
$DLL_PATH = "$SCRIPT_DIR/libmpv-2.dll"
$OUTPUT_PATH = "$SCRIPT_DIR"

Write-Host "Working with directory: $SCRIPT_DIR"
Write-Host "Looking for DLL at: $DLL_PATH"

# Create def file from our DLL
& "C:/msys64/mingw64/bin/gendef.exe" "$DLL_PATH"

# Generate lib file from def with explicit output path
& "C:/msys64/mingw64/bin/dlltool.exe" -d libmpv-2.def -l "$OUTPUT_PATH/libmpv-2.lib" -D libmpv-2.dll

# Verify file was created
if (Test-Path "$OUTPUT_PATH/libmpv-2.lib") {
    Write-Host "Successfully created libmpv-2.lib at $OUTPUT_PATH/libmpv-2.lib"
} else {
    Write-Host "ERROR: Failed to create libmpv-2.lib at $OUTPUT_PATH/libmpv-2.lib"
}

# Clean up def file
Remove-Item libmpv-2.def 