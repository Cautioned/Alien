fn main() {
    tauri_build::build();

    if cfg!(target_os = "windows") {
        use std::env;
        use std::path::PathBuf;
        use std::fs;
        use std::process::Command;
        
        // Verify we're building for 64-bit
        if env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap() != "64" {
            panic!("This application requires 64-bit Windows");
        }

        // Get the manifest directory (where Cargo.toml is)
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let manifest_path = PathBuf::from(&manifest_dir);

        // Copy DLL to root app directory
        let dll_path = manifest_path.join("lib").join("64").join("libmpv-2.dll");
        let dst_dll = manifest_path.join("libmpv-2.dll");
        let lib_path = manifest_path.join("mpv.lib");

        // Create scripts directory if it doesn't exist
        let scripts_src = manifest_path.join("scripts");
        if !scripts_src.exists() {
            if let Err(e) = fs::create_dir_all(&scripts_src) {
                println!("cargo:warning=Failed to create scripts directory: {}", e);
            } else {
                println!("cargo:warning=Created scripts directory at {:?}", scripts_src);
            }
        }

        // Copy scripts directory
        let scripts_dst = manifest_path.join("scripts");
        if scripts_src.exists() {
            if let Err(e) = fs::create_dir_all(&scripts_dst) {
                println!("cargo:warning=Failed to create scripts directory: {}", e);
            } else {
                if let Err(e) = copy_dir_all(&scripts_src, &scripts_dst) {
                    println!("cargo:warning=Failed to copy scripts directory: {}", e);
                } else {
                    println!("cargo:warning=Scripts directory copied successfully");
                    // Print the contents of the scripts directory
                    if let Ok(entries) = fs::read_dir(&scripts_dst) {
                        println!("cargo:warning=Contents of scripts directory:");
                        for entry in entries {
                            if let Ok(entry) = entry {
                                println!("cargo:warning=  {:?}", entry.path());
                            }
                        }
                    }
                }
            }
        } else {
            println!("cargo:warning=Scripts directory not found at {:?}", scripts_src);
        }

        // Only generate lib file if it doesn't exist
        if !lib_path.exists() {
            if dll_path.exists() {
                if let Err(e) = fs::copy(&dll_path, &dst_dll) {
                    println!("cargo:warning=Failed to copy libmpv-2.dll: {}", e);
                    println!("cargo:warning=Source path: {:?}", dll_path);
                    println!("cargo:warning=Destination path: {:?}", dst_dll);
                } else {
                    println!("cargo:warning=libmpv-2.dll copied successfully");
                }

                // Run generate-lib.ps1 script
                println!("cargo:warning=Generating lib file...");
                let script_path = manifest_path.join("generate-lib.ps1");
                let output = Command::new("powershell")
                    .args(["-ExecutionPolicy", "Bypass", "-File", script_path.to_str().unwrap()])
                    .current_dir(&manifest_path)
                    .output();

                match output {
                    Ok(output) => {
                        if output.status.success() {
                            println!("cargo:warning=Successfully generated lib file");
                            println!("cargo:warning=stdout: {}", String::from_utf8_lossy(&output.stdout));
                        } else {
                            println!("cargo:warning=Failed to generate lib file");
                            println!("cargo:warning=stderr: {}", String::from_utf8_lossy(&output.stderr));
                        }
                    }
                    Err(e) => {
                        println!("cargo:warning=Failed to run generate-lib.ps1: {}", e);
                    }
                }
            } else {
                println!("cargo:warning=libmpv-2.dll not found at {:?}", dll_path);
            }
        } else {
            println!("cargo:warning=mpv.lib already exists, skipping generation");
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn copy_dir_all(src: impl AsRef<std::path::Path>, dst: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
